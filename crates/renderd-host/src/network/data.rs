//! Datagram burst sender task for frame fragment transport over QUIC (RFC-0002 §12).
//!
//! Pulls encoded frames from `EncodePipeline`, fragments them into datagram-sized chunks
//! with 16-byte [`FragmentHeader`]s, and transmits all fragments of a frame in a single
//! non-yielding burst over QUIC datagrams.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bytes::{Bytes, BytesMut};
use crossbeam_channel::Receiver;
use renderd_frame::{
    FragmentFlags, FragmentHeader, FLAG_FIRST_FRAG, FLAG_KEYFRAME, FLAG_LAST_FRAG, HEADER_SIZE,
    MAX_PTS_OFFSET_US,
};
use renderd_net::{FragmentBurst, NetError};

use crate::encode::EncodedFrame;
use crate::error::HostError;

/// Fallback fragment payload size used before the path MTU is known
/// (1200-byte minimum QUIC datagram, minus QUIC framing and the 16-byte header).
pub const DEFAULT_MAX_PAYLOAD_SIZE: usize = 1150;

/// Smallest payload worth sending; below this the header overhead dominates.
const MIN_PAYLOAD_SIZE: usize = 256;

/// Consecutive datagram send failures after which the sender considers the peer gone.
const MAX_CONSECUTIVE_SEND_ERRORS: u32 = 120;

/// Frames allowed to queue up behind the sender before it skips ahead.
///
/// Skipping is only worth it once the backlog costs more latency than a keyframe
/// costs bandwidth; three frames is 50 ms at 60 fps.
const SKIP_AHEAD_BACKLOG: usize = 3;

/// Host datagram burst sender task manager.
///
/// Encapsulates frame fragmentation and transmission over QUIC datagram sockets.
#[derive(Debug, Default)]
pub struct DataSender;

impl DataSender {
    /// Creates a new `DataSender` instance.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Fragments an [`EncodedFrame`] into a series of datagram byte buffers containing 16-byte headers.
    ///
    /// # Errors
    /// Returns [`HostError::Initialization`] if header encoding fails.
    pub fn fragment_frame(
        frame: &EncodedFrame,
        max_payload_size: usize,
    ) -> Result<Vec<Bytes>, HostError> {
        let data = &frame.data;
        let total_bytes = data.len();

        let chunk_size = if max_payload_size == 0 {
            DEFAULT_MAX_PAYLOAD_SIZE
        } else {
            max_payload_size
        };

        // Determine total number of fragments (at least 1, even if data is empty)
        let frag_total_count = if total_bytes == 0 {
            1
        } else {
            total_bytes.div_ceil(chunk_size)
        };

        let frag_total = u16::try_from(frag_total_count).map_err(|_| {
            HostError::Initialization(format!(
                "Frame fragment count {frag_total_count} exceeds u16::MAX"
            ))
        })?;

        // The wire field is 24 bits of microseconds, so it wraps every ~16.7 s. Reduce
        // modulo the field width rather than converting first: a `u32::try_from` on a
        // full-range capture timestamp fails and would silently pin every frame to 0.
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let pts_offset_us = {
            let pts_us = frame.pts_ns.max(0) as u64 / 1_000;
            (pts_us % (u64::from(MAX_PTS_OFFSET_US) + 1)) as u32
        };

        let mut datagrams = Vec::with_capacity(frag_total_count);

        if total_bytes == 0 {
            let mut flags = FragmentFlags::new();
            flags.set_keyframe(frame.is_keyframe);
            flags.set_first(true);
            flags.set_last(true);

            let header = FragmentHeader {
                frame_id: frame.frame_id,
                frag_id: 0,
                frag_total,
                flags: flags.bits(),
                pts_offset_us,
            };

            let mut buf = vec![0u8; HEADER_SIZE];
            header
                .encode(&mut buf)
                .map_err(|e| HostError::Initialization(format!("Header encode error: {e}")))?;
            datagrams.push(Bytes::from(buf));
        } else {
            for (idx, chunk) in data.chunks(chunk_size).enumerate() {
                let frag_id = u16::try_from(idx).map_err(|_| {
                    HostError::Initialization("Fragment index exceeds u16::MAX".to_string())
                })?;

                let mut bits = 0u8;
                if frame.is_keyframe {
                    bits |= FLAG_KEYFRAME;
                }
                if idx == 0 {
                    bits |= FLAG_FIRST_FRAG;
                }
                if idx == frag_total_count - 1 {
                    bits |= FLAG_LAST_FRAG;
                }

                let header = FragmentHeader {
                    frame_id: frame.frame_id,
                    frag_id,
                    frag_total,
                    flags: bits,
                    pts_offset_us,
                };

                let mut packet = BytesMut::with_capacity(HEADER_SIZE + chunk.len());
                packet.resize(HEADER_SIZE, 0);

                header
                    .encode(&mut packet[..HEADER_SIZE])
                    .map_err(|e| HostError::Initialization(format!("Header encode error: {e}")))?;

                packet.extend_from_slice(chunk);
                datagrams.push(packet.freeze());
            }
        }

        Ok(datagrams)
    }

    /// Largest fragment payload the connection will carry right now.
    ///
    /// Tracks path MTU discovery: the first frames of a session go out in ~1.1 KiB
    /// fragments and, once the path is probed, in ~1.4 KiB ones — 20% fewer packets
    /// for the same video.
    #[must_use]
    pub fn payload_size_for(connection: &quinn::Connection) -> usize {
        FragmentBurst::max_datagram_size(connection)
            .map(|max| max.saturating_sub(HEADER_SIZE))
            .filter(|&payload| payload >= MIN_PAYLOAD_SIZE)
            .unwrap_or(DEFAULT_MAX_PAYLOAD_SIZE)
    }

    /// Sends encoded frame fragments over a QUIC connection in non-yielding bursts,
    /// sized to the connection's current maximum datagram size.
    ///
    /// # Errors
    /// Returns [`HostError::Initialization`] if network datagram sending fails.
    pub fn send_frame_burst(
        &self,
        connection: &quinn::Connection,
        frame: &EncodedFrame,
    ) -> Result<usize, HostError> {
        let fragments = Self::fragment_frame(frame, Self::payload_size_for(connection))?;
        FragmentBurst::send_all(connection, &fragments)
            .map_err(|e| HostError::Initialization(format!("Datagram burst send failed: {e}")))
    }

    /// Runs the datagram sender loop on the calling thread, consuming frames from `rx`
    /// and transmitting them over `connection` until `shutdown` is set, the channel is
    /// disconnected, or the QUIC connection dies.
    ///
    /// `request_keyframe` is invoked whenever the sender has to skip frames, so the
    /// encoder can resynchronise the decoder with an IDR.
    ///
    /// This blocks on the channel rather than polling it: `quinn::Connection::send_datagram`
    /// is synchronous, so there is nothing to await between frames, and a poll-and-sleep
    /// loop would add up to a whole sleep interval of latency to every frame.
    #[allow(clippy::too_many_lines)]
    pub fn run_blocking(
        &self,
        connection: &quinn::Connection,
        rx: &Receiver<EncodedFrame>,
        shutdown: &AtomicBool,
        request_keyframe: &(dyn Fn() + Sync),
    ) {
        /// Wake-up interval used only to re-check the shutdown flag while idle.
        const IDLE_POLL: std::time::Duration = std::time::Duration::from_millis(50);

        let mut sent_frames: u64 = 0;
        let mut consecutive_errors: u32 = 0;
        // Set after skipping frames: non-key frames are withheld until the encoder
        // produces the IDR that makes them decodable again.
        let mut awaiting_keyframe = false;

        let mut interval_start = std::time::Instant::now();
        let mut interval_frames: u64 = 0;
        let mut interval_bytes: u64 = 0;
        let mut interval_skipped: u64 = 0;

        while !shutdown.load(Ordering::Relaxed) {
            let mut frame = match rx.recv_timeout(IDLE_POLL) {
                Ok(frame) => frame,
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            };

            // Only skip ahead when a real backlog has built up. Skipping a single
            // queued frame breaks the decoder's reference chain and costs a keyframe,
            // which is far more expensive than the 16 ms it saves.
            if rx.len() >= SKIP_AHEAD_BACKLOG {
                let mut skipped = 0u64;
                while let Ok(fresher) = rx.try_recv() {
                    skipped += 1;
                    frame = fresher;
                }
                interval_skipped += skipped;
                if !frame.is_keyframe {
                    awaiting_keyframe = true;
                    request_keyframe();
                }
            }

            if awaiting_keyframe {
                if frame.is_keyframe {
                    awaiting_keyframe = false;
                } else {
                    // Undecodable without its reference; do not waste the link on it.
                    interval_skipped += 1;
                    continue;
                }
            }

            let frame_bytes = frame.data.len();
            let frame_id = frame.frame_id;
            let is_kf = frame.is_keyframe;
            let queue_depth = rx.len();

            match self.send_frame_burst(connection, &frame) {
                Ok(num_frags) => {
                    consecutive_errors = 0;
                    sent_frames += 1;
                    interval_frames += 1;
                    interval_bytes += frame_bytes as u64;

                    if sent_frames == 1 {
                        tracing::info!(
                            frame_id,
                            bytes = frame_bytes,
                            frags = num_frags,
                            payload_size = Self::payload_size_for(connection),
                            "DataSender: transmitted first encoded frame"
                        );
                    }

                    let elapsed = interval_start.elapsed();
                    if elapsed >= std::time::Duration::from_secs(1) {
                        let elapsed_sec = elapsed.as_secs_f64();
                        #[allow(clippy::cast_precision_loss)]
                        let fps = (interval_frames as f64) / elapsed_sec;
                        #[allow(clippy::cast_precision_loss)]
                        let instantaneous_bitrate_kbps =
                            ((interval_bytes as f64) * 8.0 / 1000.0) / elapsed_sec;
                        let avg_frame_kb = interval_bytes
                            .checked_div(interval_frames)
                            .map_or(0, |b| b / 1024);

                        tracing::info!(
                            fps = format!("{fps:.1}"),
                            bitrate_kbps = format!("{instantaneous_bitrate_kbps:.0}"),
                            avg_frame_kb,
                            skipped = interval_skipped,
                            queue_depth,
                            payload_size = Self::payload_size_for(connection),
                            rtt_ms = format!("{:.2}", connection.rtt().as_secs_f64() * 1000.0),
                            last_frame_id = frame_id,
                            is_keyframe = is_kf,
                            total_sent = sent_frames,
                            "HOST METRICS: transmit throughput"
                        );

                        interval_start = std::time::Instant::now();
                        interval_frames = 0;
                        interval_bytes = 0;
                        interval_skipped = 0;
                    }
                }
                Err(e) => {
                    consecutive_errors += 1;
                    // A fragment sized for the old path MTU was rejected mid-burst; the
                    // frame is now partial on the wire, so resynchronise with an IDR.
                    awaiting_keyframe = true;
                    request_keyframe();
                    // A dead connection fails on every frame; stop rather than spinning
                    // and logging forever for a viewer that is already gone.
                    if consecutive_errors >= MAX_CONSECUTIVE_SEND_ERRORS {
                        tracing::warn!(
                            errors = consecutive_errors,
                            "DataSender: giving up after repeated datagram send failures: {e}"
                        );
                        break;
                    }
                    tracing::warn!("Failed to send datagram burst: {e}");
                }
            }
        }

        tracing::info!(
            sent_frames,
            "DataSender: transmit loop finished for this connection"
        );
    }

    /// Runs the datagram sender loop from an async context.
    ///
    /// The work is offloaded to a blocking thread because the loop parks on a
    /// synchronous channel, which must never happen on a tokio worker thread.
    pub async fn run_loop(
        &self,
        connection: quinn::Connection,
        rx: Receiver<EncodedFrame>,
        shutdown: Arc<AtomicBool>,
        request_keyframe: Arc<dyn Fn() + Send + Sync>,
    ) {
        let sender = Self::new();
        if let Err(e) = tokio::task::spawn_blocking(move || {
            sender.run_blocking(&connection, &rx, &shutdown, &*request_keyframe);
        })
        .await
        {
            tracing::warn!("DataSender blocking task failed: {e}");
        }
    }
}

impl From<NetError> for HostError {
    fn from(e: NetError) -> Self {
        Self::Initialization(format!("network error: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fragmentation_single_fragment() {
        let frame = EncodedFrame {
            frame_id: 42,
            is_keyframe: true,
            data: Bytes::from_static(b"hello world"),
            pts_ns: 1_000_000,
        };

        let frags = DataSender::fragment_frame(&frame, 1184).unwrap();
        assert_eq!(frags.len(), 1);
        assert_eq!(frags[0].len(), HEADER_SIZE + 11);

        let header = FragmentHeader::decode(&frags[0][..HEADER_SIZE]).unwrap();
        assert_eq!(header.frame_id, 42);
        assert_eq!(header.frag_id, 0);
        assert_eq!(header.frag_total, 1);
        assert_eq!(header.flags & FLAG_KEYFRAME, FLAG_KEYFRAME);
        assert_eq!(header.flags & FLAG_FIRST_FRAG, FLAG_FIRST_FRAG);
        assert_eq!(header.flags & FLAG_LAST_FRAG, FLAG_LAST_FRAG);
    }

    #[test]
    fn test_fragmentation_multi_fragment() {
        let payload = vec![0xAB; 2500];
        let frame = EncodedFrame {
            frame_id: 100,
            is_keyframe: false,
            data: Bytes::from(payload),
            pts_ns: 16_666_666,
        };

        let frags = DataSender::fragment_frame(&frame, 1000).unwrap();
        assert_eq!(frags.len(), 3); // 1000 + 1000 + 500

        // First fragment
        let h0 = FragmentHeader::decode(&frags[0][..HEADER_SIZE]).unwrap();
        assert_eq!(h0.frag_id, 0);
        assert_eq!(h0.frag_total, 3);
        assert_eq!(h0.flags & FLAG_FIRST_FRAG, FLAG_FIRST_FRAG);
        assert_eq!(h0.flags & FLAG_LAST_FRAG, 0);

        // Second fragment
        let h1 = FragmentHeader::decode(&frags[1][..HEADER_SIZE]).unwrap();
        assert_eq!(h1.frag_id, 1);
        assert_eq!(h1.frag_total, 3);
        assert_eq!(h1.flags & FLAG_FIRST_FRAG, 0);
        assert_eq!(h1.flags & FLAG_LAST_FRAG, 0);

        // Third fragment
        let h2 = FragmentHeader::decode(&frags[2][..HEADER_SIZE]).unwrap();
        assert_eq!(h2.frag_id, 2);
        assert_eq!(h2.frag_total, 3);
        assert_eq!(h2.flags & FLAG_FIRST_FRAG, 0);
        assert_eq!(h2.flags & FLAG_LAST_FRAG, FLAG_LAST_FRAG);
    }

    /// A full-range capture timestamp must survive as a wrapped 24-bit microsecond
    /// value, not collapse to zero.
    #[test]
    fn test_large_pts_wraps_instead_of_zeroing() {
        // Mach absolute time scale: far beyond what fits in u32 microseconds.
        let pts_ns = 9_876_543_210_000i64;
        let frame = EncodedFrame {
            frame_id: 1,
            is_keyframe: true,
            data: Bytes::from_static(b"x"),
            pts_ns,
        };

        let frags = DataSender::fragment_frame(&frame, 1184).unwrap();
        let header = FragmentHeader::decode(&frags[0][..HEADER_SIZE]).unwrap();

        #[allow(clippy::cast_sign_loss)]
        let micros = pts_ns as u64 / 1_000;
        let expected = u32::try_from(micros % (u64::from(MAX_PTS_OFFSET_US) + 1))
            .expect("masked value fits in u32");
        assert_eq!(header.pts_offset_us, expected);
        assert_ne!(header.pts_offset_us, 0, "large PTS must not collapse to 0");
    }

    /// Negative or zero timestamps clamp to 0 without panicking.
    #[test]
    fn test_negative_pts_clamps_to_zero() {
        let frame = EncodedFrame {
            frame_id: 1,
            is_keyframe: true,
            data: Bytes::from_static(b"x"),
            pts_ns: -5_000_000,
        };
        let frags = DataSender::fragment_frame(&frame, 1184).unwrap();
        let header = FragmentHeader::decode(&frags[0][..HEADER_SIZE]).unwrap();
        assert_eq!(header.pts_offset_us, 0);
    }

    #[test]
    fn test_mock_loopback_burst_end_to_end() {
        let (host_mock, mut viewer_mock) = renderd_net::MockConnection::pair(10);

        let frame = EncodedFrame {
            frame_id: 1,
            is_keyframe: true,
            data: Bytes::from(vec![1, 2, 3, 4, 5]),
            pts_ns: 500_000,
        };

        let frags = DataSender::fragment_frame(&frame, 1184).unwrap();
        assert_eq!(frags.len(), 1);

        // Send via mock connection
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            host_mock.send_datagram(frags[0].clone()).await.unwrap();
            let recv = viewer_mock.recv_datagram().await.unwrap();
            assert_eq!(recv, frags[0]);
        });
    }
}
