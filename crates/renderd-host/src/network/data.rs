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
use renderd_net::FragmentBurst;

use crate::encode::EncodedFrame;
use crate::error::HostError;

/// Default maximum fragment payload size in bytes (1200 max QUIC datagram - 16-byte header).
pub const DEFAULT_MAX_PAYLOAD_SIZE: usize = 1184;

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

        let pts_offset_us =
            u32::try_from((frame.pts_ns / 1_000).max(0)).unwrap_or(0) & MAX_PTS_OFFSET_US;

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

    /// Sends encoded frame fragments over a QUIC connection in non-yielding bursts.
    ///
    /// Pulls frames from the `Receiver<EncodedFrame>` until the channel is empty or
    /// `shutdown` is signalled.
    ///
    /// # Errors
    /// Returns [`HostError::Initialization`] if network datagram sending fails.
    pub fn send_frame_burst(
        &self,
        connection: &quinn::Connection,
        frame: &EncodedFrame,
    ) -> Result<usize, HostError> {
        let fragments = Self::fragment_frame(frame, DEFAULT_MAX_PAYLOAD_SIZE)?;
        FragmentBurst::send_all(connection, &fragments)
            .map_err(|e| HostError::Initialization(format!("Datagram burst send failed: {e}")))
    }

    /// Runs the datagram sender event loop, consuming frames from `rx` and transmitting them over `conn`.
    pub async fn run_loop(
        &self,
        connection: quinn::Connection,
        rx: Receiver<EncodedFrame>,
        shutdown: Arc<AtomicBool>,
    ) {
        while !shutdown.load(Ordering::Relaxed) {
            match rx.try_recv() {
                Ok(frame) => {
                    if let Err(e) = self.send_frame_burst(&connection, &frame) {
                        tracing::warn!("Failed to send datagram burst: {e}");
                    }
                }
                Err(crossbeam_channel::TryRecvError::Empty) => {
                    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    break;
                }
            }
        }
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
