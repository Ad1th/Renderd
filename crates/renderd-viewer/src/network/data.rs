//! Datagram receiver & sliding-window reassembly task (`renderd-viewer/src/network/data.rs`).
//!
//! Receives UDP/QUIC datagrams, parses 16-byte fragment headers, feeds the sliding-window
//! `ReassemblyBuffer`, and hands completed frame bitstreams to the video decoder (RFC-0002 §12.2).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use bytes::Bytes;
use renderd_frame::{FragmentHeader, ReassemblyBuffer, HEADER_SIZE};

use crate::decoder::Decoder;
use crate::error::ViewerError;
use crate::frame_queue::FrameQueue;

/// Datagram receiver and sliding-window frame reassembly manager.
#[derive(Debug)]
pub struct DatagramReceiver {
    window: ReassemblyBuffer,
    received_datagrams: AtomicU64,
    reassembled_frames: AtomicU64,
    dropped_fragments: AtomicU64,
}

impl Default for DatagramReceiver {
    fn default() -> Self {
        Self::new(4)
    }
}

impl DatagramReceiver {
    /// Creates a new `DatagramReceiver` with target sliding window capacity (default `W = 4`).
    #[must_use]
    pub const fn new(window_size: usize) -> Self {
        Self {
            window: ReassemblyBuffer::new(window_size),
            received_datagrams: AtomicU64::new(0),
            reassembled_frames: AtomicU64::new(0),
            dropped_fragments: AtomicU64::new(0),
        }
    }

    /// Processes an incoming raw QUIC datagram payload buffer.
    ///
    /// # Errors
    /// Returns [`ViewerError::Network`] or [`ViewerError::Decoder`] if header parsing or decoding fails.
    pub fn process_datagram<D: Decoder + ?Sized>(
        &mut self,
        datagram: &[u8],
        decoder: &mut D,
    ) -> Result<Option<u64>, ViewerError> {
        self.received_datagrams.fetch_add(1, Ordering::Relaxed);

        if datagram.len() < HEADER_SIZE {
            self.dropped_fragments.fetch_add(1, Ordering::Relaxed);
            return Err(ViewerError::Network(format!(
                "Datagram length {} under 16-byte header size",
                datagram.len()
            )));
        }

        let header = FragmentHeader::decode(datagram).map_err(|err| {
            self.dropped_fragments.fetch_add(1, Ordering::Relaxed);
            ViewerError::Network(format!("Fragment header decode error: {err:?}"))
        })?;

        let payload = Bytes::copy_from_slice(&datagram[HEADER_SIZE..]);

        match self.window.insert(header, payload) {
            Ok(Some(frame)) => {
                let frame_count = self.reassembled_frames.fetch_add(1, Ordering::Relaxed) + 1;
                let pts_ns = u64::from(frame.pts_offset_us) * 1000;

                if frame_count <= 8 {
                    let payload_len = frame.payload.len();
                    let first32 = &frame.payload[..32.min(payload_len)];
                    let last32 = &frame.payload[payload_len.saturating_sub(32)..];
                    tracing::info!(
                        frame_id = frame.frame_id,
                        payload_len,
                        first32 = ?first32,
                        last32 = ?last32,
                        "REASSEMBLY: complete frame delivered to decoder"
                    );
                }

                decoder.decode_packet(&frame.payload, frame.frame_id, pts_ns)?;
                Ok(Some(frame.frame_id))
            }
            Ok(None) => Ok(None),
            Err(err) => {
                self.dropped_fragments.fetch_add(1, Ordering::Relaxed);
                Err(ViewerError::Network(format!("Reassembly error: {err:?}")))
            }
        }
    }

    /// Runs the datagram receiver event loop, reading datagrams from `connection`
    /// and passing completed frames to `decoder` and `frame_queue`.
    ///
    /// # Errors
    /// Returns [`ViewerError::Network`] if reading from QUIC connection fails.
    pub async fn run_receive_loop<D: Decoder + ?Sized>(
        &mut self,
        connection: &quinn::Connection,
        decoder: &mut D,
        frame_queue: &Arc<FrameQueue>,
    ) -> Result<(), ViewerError> {
        static RECV_DG_COUNT: AtomicU64 = AtomicU64::new(0);
        static REASM_FRAME_COUNT: AtomicU64 = AtomicU64::new(0);
        static DECODED_FRAME_COUNT: AtomicU64 = AtomicU64::new(0);

        let mut interval_start = std::time::Instant::now();
        let mut interval_datagrams: u64 = 0;
        let mut interval_reassembled: u64 = 0;
        let mut interval_bytes: u64 = 0;
        let mut interval_decoded: u64 = 0;

        while let Ok(datagram) = connection.read_datagram().await {
            let dg_len = datagram.len();
            let dg_count = RECV_DG_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
            interval_datagrams += 1;
            interval_bytes += dg_len as u64;

            if dg_count == 1 {
                tracing::info!(
                    count = dg_count,
                    bytes = dg_len,
                    "DatagramReceiver: first QUIC datagram received from host"
                );
            }

            if let Ok(Some(frame_id)) = self.process_datagram(&datagram, decoder) {
                let frame_count = REASM_FRAME_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
                interval_reassembled += 1;

                if frame_count == 1 {
                    tracing::info!(
                        count = frame_count,
                        frame_id = frame_id,
                        "DatagramReceiver: first frame reassembled & delivered to decoder"
                    );
                }

                // Drain everything the decoder has ready, not just one frame. Hardware
                // decoders deliver asynchronously, so taking exactly one output per
                // input leaves the display a fixed number of frames behind and never
                // recovers that latency after a stall.
                while let Ok(Some(decoded)) = decoder.receive_frame() {
                    let dec_count = DECODED_FRAME_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
                    interval_decoded += 1;

                    if dec_count == 1 {
                        tracing::info!(
                            count = dec_count,
                            frame_id = decoded.frame_id,
                            width = decoded.width,
                            height = decoded.height,
                            "DatagramReceiver: first decoded frame pushed into FrameQueue"
                        );
                    }
                    let _ = frame_queue.push(decoded);
                }
            }

            let elapsed = interval_start.elapsed();
            if elapsed >= std::time::Duration::from_secs(1) {
                let elapsed_sec = elapsed.as_secs_f64();
                #[allow(clippy::cast_precision_loss)]
                let dg_fps = (interval_datagrams as f64) / elapsed_sec;
                #[allow(clippy::cast_precision_loss)]
                let reasm_fps = (interval_reassembled as f64) / elapsed_sec;
                #[allow(clippy::cast_precision_loss)]
                let decoded_fps = (interval_decoded as f64) / elapsed_sec;
                #[allow(clippy::cast_precision_loss)]
                let recv_bitrate_kbps = ((interval_bytes as f64) * 8.0 / 1000.0) / elapsed_sec;

                tracing::info!(
                    recv_dg_sec = format!("{dg_fps:.1}"),
                    reasm_fps = format!("{reasm_fps:.1}"),
                    decoded_fps = format!("{decoded_fps:.1}"),
                    recv_bitrate_kbps = format!("{recv_bitrate_kbps:.0}"),
                    reasm_pending = self.window.pending_len(),
                    reasm_dropped = self.window.dropped_frames(),
                    frag_dropped = self.dropped_fragments(),
                    frame_queue_len = frame_queue.len(),
                    "VIEWER METRICS: network receive & decode throughput"
                );

                interval_start = std::time::Instant::now();
                interval_datagrams = 0;
                interval_reassembled = 0;
                interval_bytes = 0;
                interval_decoded = 0;
            }
        }
        Ok(())
    }

    /// Returns total count of received datagrams.
    #[must_use]
    pub fn received_datagrams(&self) -> u64 {
        self.received_datagrams.load(Ordering::Relaxed)
    }

    /// Returns total count of reassembled frames.
    #[must_use]
    pub fn reassembled_frames(&self) -> u64 {
        self.reassembled_frames.load(Ordering::Relaxed)
    }

    /// Returns total count of dropped fragments.
    #[must_use]
    pub fn dropped_fragments(&self) -> u64 {
        self.dropped_fragments.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decoder::NullDecoder;
    use renderd_frame::{FLAG_FIRST_FRAG, FLAG_KEYFRAME, FLAG_LAST_FRAG};

    fn encode_datagram(header: &FragmentHeader, payload: &[u8]) -> Vec<u8> {
        let mut buf = vec![0u8; HEADER_SIZE];
        header.encode(&mut buf).unwrap();
        buf.extend_from_slice(payload);
        buf
    }

    #[test]
    fn test_datagram_receiver_single_fragment_frame() {
        let mut receiver = DatagramReceiver::new(4);
        let mut decoder = NullDecoder::new();
        decoder.initialize("hevc", 1920, 1080).unwrap();

        let header = FragmentHeader {
            frame_id: 1,
            frag_id: 0,
            frag_total: 1,
            flags: FLAG_KEYFRAME | FLAG_FIRST_FRAG | FLAG_LAST_FRAG,
            pts_offset_us: 1000,
        };

        let packet = encode_datagram(&header, &[0x00, 0x00, 0x00, 0x01, 0x67]);

        let frame_res = receiver.process_datagram(&packet, &mut decoder).unwrap();
        assert_eq!(frame_res, Some(1));
        assert_eq!(receiver.received_datagrams(), 1);
        assert_eq!(receiver.reassembled_frames(), 1);
        assert_eq!(receiver.dropped_fragments(), 0);
    }

    #[test]
    fn test_datagram_receiver_multi_fragment_reassembly() {
        let mut receiver = DatagramReceiver::new(4);
        let mut decoder = NullDecoder::new();
        decoder.initialize("hevc", 1920, 1080).unwrap();

        let header1 = FragmentHeader {
            frame_id: 10,
            frag_id: 0,
            frag_total: 2,
            flags: FLAG_KEYFRAME | FLAG_FIRST_FRAG,
            pts_offset_us: 2000,
        };

        let header2 = FragmentHeader {
            frame_id: 10,
            frag_id: 1,
            frag_total: 2,
            flags: FLAG_LAST_FRAG,
            pts_offset_us: 2000,
        };

        let pkt1 = encode_datagram(&header1, &[0x01, 0x02, 0x03]);
        let pkt2 = encode_datagram(&header2, &[0x04, 0x05, 0x06]);

        let res1 = receiver.process_datagram(&pkt1, &mut decoder).unwrap();
        assert_eq!(res1, None);

        let res2 = receiver.process_datagram(&pkt2, &mut decoder).unwrap();
        assert_eq!(res2, Some(10));

        assert_eq!(receiver.received_datagrams(), 2);
        assert_eq!(receiver.reassembled_frames(), 1);
    }

    #[test]
    fn test_datagram_receiver_short_header_drop() {
        let mut receiver = DatagramReceiver::new(4);
        let mut decoder = NullDecoder::new();
        decoder.initialize("hevc", 1920, 1080).unwrap();

        let short_pkt = vec![0u8; 10]; // Under 16 bytes
        let res = receiver.process_datagram(&short_pkt, &mut decoder);
        assert!(res.is_err());
        assert_eq!(receiver.dropped_fragments(), 1);
    }
}
