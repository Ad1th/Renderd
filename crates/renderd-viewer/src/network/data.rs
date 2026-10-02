//! Datagram receiver and loss-recovering reassembly task (`renderd-viewer/src/network/data.rs`).
//!
//! Receives QUIC datagrams, parses 16-byte fragment headers, feeds the
//! [`ReceiveWindow`] — which reassembles frames, asks for lost fragments again,
//! and hands frames out in decode order — and passes those frames to the video
//! decoder (RFC-0002 §12.2).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use bytes::Bytes;
use futures_util::FutureExt as _;
use renderd_frame::{
    FragmentHeader, NackRequest, ReassembledFrame, ReceiveOutput, ReceiveWindow, HEADER_SIZE,
};

use crate::decoder::Decoder;
use crate::error::ViewerError;
use crate::frame_queue::FrameQueue;

/// Upper bound on how many already-queued datagrams a single drain pass will pull
/// before yielding back to decode. Not a normal limit — only a safety valve against
/// an unbounded loop if a peer somehow floods the connection.
const DRAIN_CAP: usize = 4096;

/// Capture-time span of the frames completed in one drain pass above which
/// decode counts as having fallen behind.
///
/// Frames routinely complete together without decode being behind at all: on
/// a slow link a small P-frame lands right behind a large one that was still
/// serialising, and Wi-Fi delivers in aggregated bursts. Decoding a few extra
/// P-frames costs a millisecond or two each on a hardware decoder; skipping
/// them costs a keyframe, which is 100-400 KB and 130-500 ms of a 6 Mbps link.
/// So only a pass holding more than this much video is a real backlog.
const BACKLOG_SPAN_NS: u64 = 150_000_000;

/// Frames in one drain pass that count as a backlog regardless of their
/// timestamps, in case a stream carries no usable ones.
const BACKLOG_FRAMES: usize = 8;

/// Whether the frames completed in one drain pass show decode falling behind.
///
/// `pts_ns` are the completed frames' capture timestamps in arrival order.
#[must_use]
pub fn is_decode_backlog(pts_ns: &[u64]) -> bool {
    if pts_ns.len() > BACKLOG_FRAMES {
        return true;
    }
    let (Some(min), Some(max)) = (pts_ns.iter().min(), pts_ns.iter().max()) else {
        return false;
    };
    max - min > BACKLOG_SPAN_NS
}

/// What the receive loop is telling the control-plane feedback task.
///
/// Loss and a decode backlog must stay distinct: both can end in a keyframe
/// request, but only one of them is evidence the *network* is struggling.
/// Conflating them (as a single loss counter previously did) meant that every
/// local, CPU-bound decode backlog got reported to the host as packet loss and
/// pulled the encoder bitrate down for no network reason at all.
#[derive(Debug, Clone)]
pub enum RecoverySignal {
    /// Frames were lost for good: fragments missing past every retransmit
    /// attempt. Real evidence of network trouble, counted toward the reported
    /// loss rate. Says nothing about keyframes; [`Self::KeyframeNeeded`] does.
    FragmentLoss(u64),
    /// Fragments to ask the host to send again.
    Nack(Vec<NackRequest>),
    /// A reference frame was lost and nothing after it can be decoded until a
    /// keyframe arrives.
    KeyframeNeeded,
    /// Decode fell behind datagram arrival and frames were skipped to catch up.
    /// A purely local, CPU-bound event; it asks for a keyframe but must never be
    /// reported as loss.
    DecodeBacklog,
    /// A whole frame finished arriving. Feeds the one-way queuing-delay and
    /// receive-rate estimate; it says nothing about loss.
    FrameArrived {
        /// Host capture timestamp of the frame, reconstructed to absolute nanoseconds.
        pts_ns: u64,
        /// Encoded size of the frame.
        bytes: usize,
        /// When the datagram batch that completed the frame was read.
        arrival: std::time::Instant,
    },
    /// A frame was successfully decoded. This is what actually drives the loss
    /// rate the host's ABR loop reacts to: `FeedbackExporter::record_frame`
    /// tracks `frame_id` gaps itself, so without this signal `received_frames`
    /// never advances.
    FrameDecoded {
        /// Frame sequence identifier of the frame that was just decoded.
        frame_id: u64,
        /// Time the hardware decoder spent on this frame.
        decode_duration: std::time::Duration,
    },
}

/// Datagram receiver: reassembly, loss recovery and decode ordering.
#[derive(Debug)]
pub struct DatagramReceiver {
    window: ReceiveWindow,
    output: ReceiveOutput,
    received_datagrams: AtomicU64,
    reassembled_frames: AtomicU64,
    dropped_fragments: AtomicU64,
    /// Most recently reconstructed absolute timestamp, in microseconds.
    /// `None` until the first frame.
    last_pts_us: Option<u64>,
}

impl Default for DatagramReceiver {
    fn default() -> Self {
        Self::new()
    }
}

impl DatagramReceiver {
    /// Creates a receiver waiting for its first keyframe.
    #[must_use]
    pub fn new() -> Self {
        Self {
            window: ReceiveWindow::default(),
            output: ReceiveOutput::default(),
            received_datagrams: AtomicU64::new(0),
            reassembled_frames: AtomicU64::new(0),
            dropped_fragments: AtomicU64::new(0),
            last_pts_us: None,
        }
    }

    /// Reconstructs an absolute presentation timestamp, in nanoseconds, from the
    /// wire's 24-bit wrapping microsecond field.
    ///
    /// The field wraps every `MAX_PTS_OFFSET_US + 1` microseconds (~16.777 s).
    /// Fed to a platform decoder unwrapped, every stream would see a hard
    /// ~16.7 s backward jump at every wrap boundary — decoders that validate or
    /// reorder on presentation time (Media Foundation's `IMFSample::SetSampleTime`
    /// among them) can stall or glitch right there. Each raw value is placed in
    /// whichever wrap period puts it nearest the last timestamp seen. Frames
    /// are at most a few hundred milliseconds apart, even when one is held back
    /// for a retransmit, far less than half a period, so the nearest placement
    /// is always the right one — before or after a wrap.
    fn unwrap_pts_ns(&mut self, raw_us: u32) -> u64 {
        const PERIOD_US: u64 = renderd_frame::MAX_PTS_OFFSET_US as u64 + 1;

        let raw = u64::from(raw_us);
        let abs = self.last_pts_us.map_or(raw, |last| {
            let base = last - last % PERIOD_US;
            [
                base.checked_sub(PERIOD_US),
                Some(base),
                Some(base + PERIOD_US),
            ]
            .into_iter()
            .flatten()
            .map(|period| period + raw)
            .min_by_key(|&candidate| candidate.abs_diff(last))
            .unwrap_or(base + raw)
        });
        match self.last_pts_us {
            Some(last) if last >= abs => {}
            _ => self.last_pts_us = Some(abs),
        }
        abs * 1000
    }

    /// Processes one raw datagram and decodes whatever it makes ready.
    ///
    /// Returns the id of the last frame decoded, if any.
    ///
    /// # Errors
    /// Returns [`ViewerError::Network`] if the datagram is malformed, or
    /// [`ViewerError::Decoder`] if decoding fails.
    pub fn process_datagram<D: Decoder + ?Sized>(
        &mut self,
        datagram: &[u8],
        decoder: &mut D,
    ) -> Result<Option<u64>, ViewerError> {
        let mut out = std::mem::take(&mut self.output);
        out.clear();
        let result = self.ingest(datagram, Instant::now(), &mut out);
        let frames = std::mem::take(&mut out.frames);
        self.output = out;
        result?;
        let mut last = None;
        for frame in frames {
            let pts_ns = self.unwrap_pts_ns(frame.pts_offset_us);
            decoder.decode_packet(&frame.payload, frame.frame_id, pts_ns)?;
            last = Some(frame.frame_id);
        }
        Ok(last)
    }

    /// Parses one raw datagram and feeds it to the receive window.
    ///
    /// # Errors
    /// Returns [`ViewerError::Network`] if the header is malformed or disagrees
    /// with earlier fragments of its frame.
    fn ingest(
        &mut self,
        datagram: &[u8],
        now: Instant,
        out: &mut ReceiveOutput,
    ) -> Result<(), ViewerError> {
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
        let arrived_before = out.arrived.len();
        self.window
            .insert(header, payload, now, out)
            .map_err(|err| {
                self.dropped_fragments.fetch_add(1, Ordering::Relaxed);
                ViewerError::Network(format!("Reassembly error: {err:?}"))
            })?;

        let completed = (out.arrived.len() - arrived_before) as u64;
        if completed > 0 {
            let frame_count = self
                .reassembled_frames
                .fetch_add(completed, Ordering::Relaxed)
                + completed;
            if frame_count <= 8 {
                tracing::info!(
                    frame_id = header.frame_id,
                    frame_count,
                    "REASSEMBLY: frame complete"
                );
            }
        }
        Ok(())
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
        self.run_receive_loop_with_loss_signal(connection, decoder, frame_queue, None)
            .await
    }

    /// Like [`Self::run_receive_loop_with_loss_signal`], but also invokes `on_frame`
    /// each time one or more decoded frames are pushed into the queue.
    ///
    /// The viewer uses this to wake its event loop exactly when there is something new
    /// to present, instead of polling for frames on a spin loop.
    ///
    /// # Errors
    /// Returns [`ViewerError::Network`] if reading from the QUIC connection fails.
    pub async fn run_receive_loop_with_wake<D, W>(
        &mut self,
        connection: &quinn::Connection,
        decoder: &mut D,
        frame_queue: &Arc<FrameQueue>,
        loss_tx: Option<tokio::sync::mpsc::Sender<RecoverySignal>>,
        on_frame: &W,
    ) -> Result<(), ViewerError>
    where
        D: Decoder + ?Sized,
        W: Fn() + Sync,
    {
        self.receive_loop_inner(connection, decoder, frame_queue, loss_tx, Some(on_frame))
            .await
    }

    /// Runs the datagram receiver event loop with an optional channel for loss,
    /// retransmit and keyframe signals.
    ///
    /// # Errors
    /// Returns [`ViewerError::Network`] if reading from QUIC connection fails.
    pub async fn run_receive_loop_with_loss_signal<D: Decoder + ?Sized>(
        &mut self,
        connection: &quinn::Connection,
        decoder: &mut D,
        frame_queue: &Arc<FrameQueue>,
        loss_tx: Option<tokio::sync::mpsc::Sender<RecoverySignal>>,
    ) -> Result<(), ViewerError> {
        self.receive_loop_inner::<D, fn()>(connection, decoder, frame_queue, loss_tx, None)
            .await
    }

    /// Runs the receive loop.
    ///
    /// # Loss
    ///
    /// Every fragment goes through the [`ReceiveWindow`]. A lost fragment is
    /// noticed as soon as anything after it arrives and is asked for again
    /// right away ([`RecoverySignal::Nack`]); the frames behind it wait, in
    /// order, for the retransmit rather than being decoded against a missing
    /// reference. The loop also wakes on the window's own deadline, so an
    /// unanswered request is repeated and a frame that never arrives is given
    /// up — the only case that costs a keyframe ([`RecoverySignal::KeyframeNeeded`]).
    ///
    /// # Standing latency
    ///
    /// A synchronous decode that runs even slightly slower than the incoming
    /// frame rate used to leave every datagram queued inside `quinn`'s own
    /// receive buffer, decoded strictly in arrival order, falling further behind
    /// the live desktop the longer the stream ran. Each pass therefore blocks for
    /// the next datagram, then drains everything already queued with a
    /// non-blocking poll, feeds it all to the window, and only then decides
    /// whether decode has fallen behind. The signal is how much *video time*
    /// finished arriving in the pass (see [`is_decode_backlog`]); raw datagram
    /// count would not do, because one frame's fragments routinely land as
    /// dozens of queued datagrams on every healthy frame. Frames completed by a
    /// retransmit are left out of that measure: their capture time is older by
    /// the recovery round trip, which says nothing about decode. On a backlog,
    /// decoding restarts from the newest keyframe in the pass, or, with none,
    /// waits for the next one and asks for it.
    #[allow(clippy::too_many_lines)]
    async fn receive_loop_inner<D, W>(
        &mut self,
        connection: &quinn::Connection,
        decoder: &mut D,
        frame_queue: &Arc<FrameQueue>,
        loss_tx: Option<tokio::sync::mpsc::Sender<RecoverySignal>>,
        on_frame: Option<&W>,
    ) -> Result<(), ViewerError>
    where
        D: Decoder + ?Sized,
        W: Fn() + Sync,
    {
        static RECV_DG_COUNT: AtomicU64 = AtomicU64::new(0);
        static DECODED_FRAME_COUNT: AtomicU64 = AtomicU64::new(0);

        let signal = |s: RecoverySignal| {
            if let Some(ref tx) = loss_tx {
                let _ = tx.try_send(s);
            }
        };

        let mut interval_start = std::time::Instant::now();
        let mut interval_datagrams: u64 = 0;
        let mut interval_reassembled: u64 = 0;
        let mut interval_bytes: u64 = 0;
        let mut interval_decoded: u64 = 0;
        let mut interval_skipped: u64 = 0;
        let mut interval_backlog_events: u64 = 0;
        let mut interval_nacked: u64 = 0;

        let mut batch: Vec<Bytes> = Vec::with_capacity(64);
        let mut out = ReceiveOutput::default();

        loop {
            // Sleep until a datagram arrives or the window has a retransmit or
            // give-up deadline to act on; nothing else wakes this task.
            let deadline = self.window.next_deadline();
            let first = tokio::select! {
                biased;
                datagram = connection.read_datagram() => match datagram {
                    Ok(datagram) => Some(datagram),
                    Err(_) => break,
                },
                () = sleep_until(deadline) => None,
            };

            let now = std::time::Instant::now();
            self.window.set_rtt(connection.rtt());
            out.clear();
            batch.clear();
            if let Some(first) = first {
                batch.push(first);
                // Pull anything already sitting in quinn's receive queue without
                // waiting. `now_or_never` polls the freshly constructed future
                // exactly once and gives up instantly if nothing is ready.
                while batch.len() < DRAIN_CAP {
                    match connection.read_datagram().now_or_never() {
                        Some(Ok(more)) => batch.push(more),
                        _ => break,
                    }
                }
            }

            for datagram in &batch {
                let dg_count = RECV_DG_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
                interval_datagrams += 1;
                interval_bytes += datagram.len() as u64;
                if dg_count == 1 {
                    tracing::info!(
                        bytes = datagram.len(),
                        "DatagramReceiver: first QUIC datagram received from host"
                    );
                }
                if let Err(e) = self.ingest(datagram, now, &mut out) {
                    tracing::debug!("dropping malformed datagram: {e}");
                }
            }
            self.window.poll(now, &mut out);

            // Report what finished arriving, and measure the pass for backlog.
            let mut arrived_pts: Vec<u64> = Vec::with_capacity(out.arrived.len());
            for arrived in &out.arrived {
                interval_reassembled += 1;
                let pts_ns = self.unwrap_pts_ns(arrived.pts_offset_us);
                if arrived.recovered {
                    continue;
                }
                arrived_pts.push(pts_ns);
                signal(RecoverySignal::FrameArrived {
                    pts_ns,
                    bytes: arrived.bytes,
                    arrival: now,
                });
            }
            if !out.nacks.is_empty() {
                interval_nacked += out
                    .nacks
                    .iter()
                    .map(|n| n.frag_ids.len().max(1) as u64)
                    .sum::<u64>();
                signal(RecoverySignal::Nack(std::mem::take(&mut out.nacks)));
            }
            if out.lost_frames > 0 {
                signal(RecoverySignal::FragmentLoss(out.lost_frames));
            }
            if out.need_keyframe {
                signal(RecoverySignal::KeyframeNeeded);
            }

            let mut frames: Vec<ReassembledFrame> = std::mem::take(&mut out.frames);
            if is_decode_backlog(&arrived_pts) {
                interval_backlog_events += 1;
                // Everything from the newest keyframe on is still decodable;
                // without one, skip to the next keyframe and ask for it.
                let restart = frames.iter().rposition(|f| f.is_keyframe);
                let skip = restart.unwrap_or(frames.len());
                interval_skipped += skip as u64;
                frames.drain(..skip);
                if restart.is_none() {
                    tracing::warn!(
                        complete_frames = arrived_pts.len(),
                        "DatagramReceiver: arrival outran decode — skipping to the next keyframe"
                    );
                    self.window.require_keyframe(now);
                    signal(RecoverySignal::DecodeBacklog);
                }
            }

            let mut pushed_any = false;
            for frame in frames {
                let pts_ns = self.unwrap_pts_ns(frame.pts_offset_us);
                if let Err(e) = decoder.decode_packet(&frame.payload, frame.frame_id, pts_ns) {
                    // Whatever comes next references this frame.
                    tracing::warn!("decode_packet failed for frame {}: {e}", frame.frame_id);
                    self.window.require_keyframe(now);
                    signal(RecoverySignal::KeyframeNeeded);
                    break;
                }

                // Drain everything the decoder has ready, not just one frame.
                while let Ok(Some(decoded)) = decoder.receive_frame() {
                    let dec_count = DECODED_FRAME_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
                    interval_decoded += 1;
                    if dec_count == 1 {
                        tracing::info!(
                            frame_id = decoded.frame_id,
                            width = decoded.width,
                            height = decoded.height,
                            "DatagramReceiver: first decoded frame pushed into FrameQueue"
                        );
                    }
                    signal(RecoverySignal::FrameDecoded {
                        frame_id: decoded.frame_id,
                        decode_duration: decoded.decode_duration,
                    });
                    let _ = frame_queue.push(decoded);
                    pushed_any = true;
                }
            }
            if pushed_any {
                if let Some(wake) = on_frame {
                    wake();
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
                    nacked_frags = interval_nacked,
                    skipped_stale = interval_skipped,
                    backlog_events = interval_backlog_events,
                    awaiting_keyframe = self.window.awaiting_keyframe(),
                    held_frames = self.window.held_frames(),
                    recovered_total = self.window.recovered_frames(),
                    lost_total = self.window.lost_frames(),
                    frag_dropped = self.dropped_fragments(),
                    frame_queue_len = frame_queue.len(),
                    "VIEWER METRICS: network receive & decode throughput"
                );

                interval_start = std::time::Instant::now();
                interval_datagrams = 0;
                interval_reassembled = 0;
                interval_bytes = 0;
                interval_decoded = 0;
                interval_skipped = 0;
                interval_backlog_events = 0;
                interval_nacked = 0;
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

/// Sleeps until `deadline`, or forever without one.
async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(at) => tokio::time::sleep_until(at.into()).await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decoder::{DecodedFrame, NullDecoder};
    use renderd_frame::{FLAG_FIRST_FRAG, FLAG_KEYFRAME, FLAG_LAST_FRAG};

    fn encode_datagram(header: &FragmentHeader, payload: &[u8]) -> Vec<u8> {
        let mut buf = vec![0u8; HEADER_SIZE];
        header.encode(&mut buf).unwrap();
        buf.extend_from_slice(payload);
        buf
    }

    #[test]
    fn test_datagram_receiver_single_fragment_frame() {
        let mut receiver = DatagramReceiver::new();
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
        let mut receiver = DatagramReceiver::new();
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
        let mut receiver = DatagramReceiver::new();
        let mut decoder = NullDecoder::new();
        decoder.initialize("hevc", 1920, 1080).unwrap();

        let short_pkt = vec![0u8; 10]; // Under 16 bytes
        let res = receiver.process_datagram(&short_pkt, &mut decoder);
        assert!(res.is_err());
        assert_eq!(receiver.dropped_fragments(), 1);
    }

    /// Test-only decoder that records the `pts_ns` it was asked to decode with,
    /// so a test can assert the reconstructed timeline is monotonically
    /// increasing across a wire wraparound, not sawtoothing.
    #[derive(Debug, Default)]
    struct PtsRecordingDecoder {
        seen_pts_ns: Vec<u64>,
    }

    impl Decoder for PtsRecordingDecoder {
        fn initialize(
            &mut self,
            _codec: &str,
            _width: u32,
            _height: u32,
        ) -> Result<(), ViewerError> {
            Ok(())
        }
        fn decode_packet(
            &mut self,
            _packet: &[u8],
            _frame_id: u64,
            pts_ns: u64,
        ) -> Result<(), ViewerError> {
            self.seen_pts_ns.push(pts_ns);
            Ok(())
        }
        fn receive_frame(&mut self) -> Result<Option<DecodedFrame>, ViewerError> {
            Ok(None)
        }
        fn reset(&mut self) -> Result<(), ViewerError> {
            Ok(())
        }
    }

    /// The wire's `pts_offset_us` is a 24-bit counter that wraps every ~16.777 s.
    /// Without unwrapping, a session running past that boundary would feed the
    /// platform decoder a hard ~16.7 s backward jump at every wrap, forever, for
    /// the life of the stream. The reconstructed timeline must stay monotonic
    /// across a wrap.
    #[test]
    fn test_pts_reconstructs_monotonic_timeline_across_wraparound() {
        let mut receiver = DatagramReceiver::new();
        let mut decoder = PtsRecordingDecoder::default();
        decoder.initialize("hevc", 1920, 1080).unwrap();

        let near_wrap = renderd_frame::MAX_PTS_OFFSET_US - 1_000; // just before wrap
        let just_wrapped = 500u32; // wire value after wrapping past 0

        for (frame_id, raw_us) in [(1u64, near_wrap), (2, just_wrapped)] {
            let mut flags = renderd_frame::FragmentFlags::new();
            flags.set_first(true);
            flags.set_last(true);
            flags.set_keyframe(true);
            let header = FragmentHeader {
                frame_id,
                frag_id: 0,
                frag_total: 1,
                flags: flags.bits(),
                pts_offset_us: raw_us,
            };
            let mut buf = vec![0u8; HEADER_SIZE];
            header.encode(&mut buf).unwrap();
            buf.extend_from_slice(&[0xAB; 4]);
            receiver.process_datagram(&buf, &mut decoder).unwrap();
        }

        assert_eq!(decoder.seen_pts_ns.len(), 2);
        assert!(
            decoder.seen_pts_ns[1] > decoder.seen_pts_ns[0],
            "timestamp must keep increasing across a wire wraparound, not jump \
             backward ~16.7s: got {:?}",
            decoder.seen_pts_ns
        );
        // The gap should be small (a couple thousand microseconds), not a full
        // ~16.7s backward jump.
        let gap_ns = decoder.seen_pts_ns[1] - decoder.seen_pts_ns[0];
        assert!(
            gap_ns < 100_000_000,
            "expected a small forward gap across the wrap, got {gap_ns} ns"
        );
    }

    /// Test-only decoder that records the id of every frame it is actually asked
    /// to decode, so a test can assert on *which* frames a backlog skipped.
    #[derive(Debug, Default, Clone)]
    struct RecordingDecoder {
        decoded_ids: std::sync::Arc<std::sync::Mutex<Vec<u64>>>,
        pending: std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<DecodedFrame>>>,
    }

    impl Decoder for RecordingDecoder {
        fn initialize(
            &mut self,
            _codec: &str,
            _width: u32,
            _height: u32,
        ) -> Result<(), ViewerError> {
            Ok(())
        }

        fn decode_packet(
            &mut self,
            _packet: &[u8],
            frame_id: u64,
            pts_ns: u64,
        ) -> Result<(), ViewerError> {
            self.decoded_ids.lock().unwrap().push(frame_id);
            self.pending.lock().unwrap().push_back(DecodedFrame {
                frame_id,
                pts_ns,
                width: 4,
                height: 4,
                format: crate::decoder::PixelFormat::Bgra8,
                buffer: vec![0u8; 64],
                decode_duration: std::time::Duration::ZERO,
                gpu: None,
            });
            Ok(())
        }

        fn receive_frame(&mut self) -> Result<Option<DecodedFrame>, ViewerError> {
            Ok(self.pending.lock().unwrap().pop_front())
        }

        fn reset(&mut self) -> Result<(), ViewerError> {
            Ok(())
        }
    }

    /// Builds a loopback QUIC connection pair for a real end-to-end receive-loop test.
    async fn loopback_pair() -> (quinn::Connection, quinn::Connection) {
        let cert_gen =
            rcgen::generate_simple_self_signed(vec!["renderd-test".to_string()]).unwrap();
        let cert_der = rustls::pki_types::CertificateDer::from(cert_gen.cert.der().to_vec());
        let key_der =
            rustls::pki_types::PrivateKeyDer::Pkcs8(cert_gen.key_pair.serialize_der().into());

        let server_tls =
            renderd_net::ServerTlsConfig::from_cert(vec![cert_der], key_der, None).unwrap();
        let client_tls = renderd_net::ClientTlsConfig::with_insecure_skip_verify().unwrap();

        let server =
            renderd_net::QuicServer::bind("127.0.0.1:0".parse().unwrap(), server_tls).unwrap();
        let addr = server.local_addr().unwrap();
        let client = renderd_net::QuicClient::bind_ephemeral().unwrap();

        let accept = tokio::spawn(async move { server.accept().await.unwrap() });
        let client_conn = client
            .connect(addr, "renderd-test", client_tls)
            .await
            .unwrap();
        let server_conn = accept.await.unwrap();
        (server_conn, client_conn)
    }

    /// One single-fragment frame captured at `frame_id` × 16.667 ms, as a 60 fps
    /// stream would stamp it.
    fn frame_datagram(frame_id: u64, is_keyframe: bool) -> Bytes {
        let mut flags = renderd_frame::FragmentFlags::new();
        flags.set_first(true);
        flags.set_last(true);
        flags.set_keyframe(is_keyframe);
        let header = FragmentHeader {
            frame_id,
            frag_id: 0,
            frag_total: 1,
            flags: flags.bits(),
            pts_offset_us: u32::try_from(frame_id * 16_667).unwrap(),
        };
        let mut buf = vec![0u8; HEADER_SIZE];
        header.encode(&mut buf).unwrap();
        buf.extend_from_slice(&[0xAB; 8]);
        Bytes::from(buf)
    }

    /// Splits one frame across `frag_total` datagrams — what a real frame looks
    /// like on the wire (the host bursts dozens of fragments per frame in one
    /// non-yielding send), unlike [`frame_datagram`]'s single-fragment shortcut.
    fn frame_fragments(frame_id: u64, is_keyframe: bool, frag_total: u16) -> Vec<Bytes> {
        (0..frag_total)
            .map(|frag_id| {
                let mut flags = renderd_frame::FragmentFlags::new();
                flags.set_first(frag_id == 0);
                flags.set_last(frag_id == frag_total - 1);
                flags.set_keyframe(is_keyframe);
                let header = FragmentHeader {
                    frame_id,
                    frag_id,
                    frag_total,
                    flags: flags.bits(),
                    pts_offset_us: 0,
                };
                let mut buf = vec![0u8; HEADER_SIZE];
                header.encode(&mut buf).unwrap();
                buf.extend_from_slice(&[0xAB; 4]);
                Bytes::from(buf)
            })
            .collect()
    }

    /// The bug this guards against: a *single* frame's fragments must never be
    /// mistaken for a decode backlog.
    ///
    /// The host bursts every fragment of one frame in one non-yielding send
    /// (`FragmentBurst::send_all`), so by the time the receive loop wakes for the
    /// first fragment and drains what else is already queued, tens of datagrams
    /// are routinely sitting there — all belonging to the *one* frame currently
    /// arriving. Treating that raw datagram count as "backlog" (an earlier version
    /// of this loop did exactly that) meant every single healthy frame looked
    /// like arrival had outrun decode, so non-keyframes were skipped permanently
    /// and the viewer never showed anything. The signal has to be how many whole
    /// *frames* came out of one drain pass, not how many datagrams did.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_single_frame_fragment_burst_is_not_mistaken_for_backlog() {
        let (host_conn, viewer_conn) = loopback_pair().await;

        let mut receiver = DatagramReceiver::new();
        let decoder = RecordingDecoder::default();
        let mut decoder_handle = decoder.clone();
        let frame_queue = Arc::new(FrameQueue::new(8));
        let (loss_tx, mut loss_rx) = tokio::sync::mpsc::channel::<RecoverySignal>(64);

        let recv_task = tokio::spawn(async move {
            let _ = receiver
                .run_receive_loop_with_loss_signal(
                    &viewer_conn,
                    &mut decoder_handle,
                    &frame_queue,
                    Some(loss_tx),
                )
                .await;
        });
        let signals = Arc::new(std::sync::Mutex::new(Vec::<RecoverySignal>::new()));
        let signals_handle = signals.clone();
        tokio::spawn(async move {
            while let Some(signal) = loss_rx.recv().await {
                signals_handle.lock().unwrap().push(signal);
            }
        });

        // One keyframe, split into 60 fragments — comparable to the ~73-98
        // fragments a real 1080p keyframe splits into — sent in one burst with
        // no gap between fragments, exactly like the real sender.
        for fragment in frame_fragments(1, true, 60) {
            host_conn.send_datagram(fragment).unwrap();
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        host_conn.close(0u32.into(), b"test done");
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), recv_task).await;

        let decoded_frame_ids = decoder.decoded_ids.lock().unwrap().clone();
        assert_eq!(
            decoded_frame_ids,
            vec![1],
            "the single frame must decode normally, not be treated as a backlog"
        );

        let recorded_signals = signals.lock().unwrap().clone();
        let non_decode_signals: Vec<_> = recorded_signals
            .iter()
            .filter(|s| {
                !matches!(
                    s,
                    RecoverySignal::FrameDecoded { .. } | RecoverySignal::FrameArrived { .. }
                )
            })
            .collect();
        assert!(
            non_decode_signals.is_empty(),
            "one frame's fragment burst must never signal a backlog: {non_decode_signals:?}"
        );
    }

    #[test]
    fn test_backlog_is_measured_in_video_time() {
        let at = |ms: u64| ms * 1_000_000;
        assert!(!is_decode_backlog(&[]));
        assert!(!is_decode_backlog(&[at(0)]));
        // Two or three frames bunched by the network: not a backlog.
        assert!(!is_decode_backlog(&[at(0), at(17), at(33)]));
        assert!(!is_decode_backlog(&[at(0), at(150)]));
        assert!(is_decode_backlog(&[at(0), at(151)]));
        // Too many frames counts even without usable timestamps.
        assert!(is_decode_backlog(&[0; BACKLOG_FRAMES + 1]));
        assert!(!is_decode_backlog(&[0; BACKLOG_FRAMES]));
    }

    /// Frames that merely arrive together — a small P-frame right behind a large
    /// one on a slow link — must all be decoded, and must not cost a keyframe.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_receive_loop_decodes_a_short_bunch_without_a_keyframe_request() {
        let (host_conn, viewer_conn) = loopback_pair().await;

        let mut receiver = DatagramReceiver::new();
        let decoder = RecordingDecoder::default();
        let mut decoder_handle = decoder.clone();
        let frame_queue = Arc::new(FrameQueue::new(8));
        let (loss_tx, mut loss_rx) = tokio::sync::mpsc::channel::<RecoverySignal>(64);

        let recv_task = tokio::spawn(async move {
            let _ = receiver
                .run_receive_loop_with_loss_signal(
                    &viewer_conn,
                    &mut decoder_handle,
                    &frame_queue,
                    Some(loss_tx),
                )
                .await;
        });
        let signals = Arc::new(std::sync::Mutex::new(Vec::<RecoverySignal>::new()));
        let signals_handle = signals.clone();
        tokio::spawn(async move {
            while let Some(signal) = loss_rx.recv().await {
                signals_handle.lock().unwrap().push(signal);
            }
        });

        host_conn.send_datagram(frame_datagram(1, true)).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        for id in 2..=4u64 {
            host_conn.send_datagram(frame_datagram(id, false)).unwrap();
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        host_conn.close(0u32.into(), b"test done");
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), recv_task).await;

        assert_eq!(*decoder.decoded_ids.lock().unwrap(), vec![1, 2, 3, 4]);
        let recorded = signals.lock().unwrap().clone();
        assert!(
            !recorded
                .iter()
                .any(|s| matches!(s, RecoverySignal::DecodeBacklog)),
            "a three-frame bunch is not a backlog: {recorded:?}"
        );
    }

    /// The regression test for the multi-second lag this module exists to fix:
    /// once arrival outruns consumption, the receive loop must skip non-keyframe
    /// backlog rather than faithfully decoding every stale frame in order.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_receive_loop_skips_stale_backlog_until_next_keyframe() {
        let (host_conn, viewer_conn) = loopback_pair().await;

        let mut receiver = DatagramReceiver::new();
        let decoder = RecordingDecoder::default();
        let mut decoder_handle = decoder.clone();
        let frame_queue = Arc::new(FrameQueue::new(8));
        let (loss_tx, mut loss_rx) = tokio::sync::mpsc::channel::<RecoverySignal>(64);

        let recv_task = tokio::spawn(async move {
            let _ = receiver
                .run_receive_loop_with_loss_signal(
                    &viewer_conn,
                    &mut decoder_handle,
                    &frame_queue,
                    Some(loss_tx),
                )
                .await;
        });
        // Collect every recovery signal the loop sends, so the test can assert on
        // *which* kind fired — not just drain them.
        let signals = Arc::new(std::sync::Mutex::new(Vec::<RecoverySignal>::new()));
        let signals_handle = signals.clone();
        tokio::spawn(async move {
            while let Some(signal) = loss_rx.recv().await {
                signals_handle.lock().unwrap().push(signal);
            }
        });

        // Frame 1 (keyframe) arrives alone and should decode immediately — no
        // backlog exists yet.
        host_conn.send_datagram(frame_datagram(1, true)).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        // Frames 2..=14 (non-key) and 15 (key) and 16..=20 (non-key) all land
        // before the receive loop can drain them one at a time — simulating decode
        // that has fallen behind real-time arrival.
        for id in 2..=20u64 {
            host_conn
                .send_datagram(frame_datagram(id, id == 15))
                .unwrap();
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        host_conn.close(0u32.into(), b"test done");
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), recv_task).await;

        let decoded_frame_ids = decoder.decoded_ids.lock().unwrap().clone();

        assert!(
            decoded_frame_ids.contains(&1),
            "the lone first keyframe must always decode: {decoded_frame_ids:?}"
        );
        assert!(
            decoded_frame_ids.contains(&15),
            "the keyframe that ends the backlog must decode: {decoded_frame_ids:?}"
        );
        assert!(
            decoded_frame_ids.iter().any(|&id| (16..=20).contains(&id)),
            "decoding must resume after the keyframe: {decoded_frame_ids:?}"
        );
        let skipped_in_backlog = (2..15).filter(|id| !decoded_frame_ids.contains(id)).count();
        assert!(
            skipped_in_backlog > 0,
            "at least some of the stale non-key backlog must be skipped, not decoded \
             in strict arrival order: decoded={decoded_frame_ids:?}"
        );

        // Not one byte was actually lost on this loopback — every datagram sent
        // above arrived. The backlog is a purely local, CPU-bound event, so it
        // must never be reported as FragmentLoss: doing so would tell the host's
        // ABR loop the network is failing and pull the bitrate down for a problem
        // more bitrate cannot fix, visibly hurting quality under exactly the
        // motion (scrolling, video) that causes backlogs in the first place.
        let recorded_signals = signals.lock().unwrap().clone();
        assert!(
            recorded_signals
                .iter()
                .all(|s| !matches!(s, RecoverySignal::FragmentLoss(_))),
            "no fragment was actually lost, so no signal may report FragmentLoss: {recorded_signals:?}"
        );
        // Frame 15 was in hand, so decoding restarted from it: asking the host
        // for yet another keyframe would only put a second one on the link.
        assert!(
            recorded_signals.iter().all(|s| !matches!(
                s,
                RecoverySignal::DecodeBacklog | RecoverySignal::KeyframeNeeded
            )),
            "the backlog already carried a keyframe: {recorded_signals:?}"
        );
    }

    /// A backlog with no keyframe in it has nothing to restart from: the loop
    /// must wait for the next keyframe, decode nothing until then, and ask.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_backlog_without_a_keyframe_asks_for_one() {
        let (host_conn, viewer_conn) = loopback_pair().await;

        let mut receiver = DatagramReceiver::new();
        let decoder = RecordingDecoder::default();
        let mut decoder_handle = decoder.clone();
        let frame_queue = Arc::new(FrameQueue::new(8));
        let (loss_tx, mut loss_rx) = tokio::sync::mpsc::channel::<RecoverySignal>(64);

        let recv_task = tokio::spawn(async move {
            let _ = receiver
                .run_receive_loop_with_loss_signal(
                    &viewer_conn,
                    &mut decoder_handle,
                    &frame_queue,
                    Some(loss_tx),
                )
                .await;
        });
        let signals = Arc::new(std::sync::Mutex::new(Vec::<RecoverySignal>::new()));
        let signals_handle = signals.clone();
        tokio::spawn(async move {
            while let Some(signal) = loss_rx.recv().await {
                signals_handle.lock().unwrap().push(signal);
            }
        });

        host_conn.send_datagram(frame_datagram(1, true)).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        for id in 2..=14u64 {
            host_conn.send_datagram(frame_datagram(id, false)).unwrap();
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        host_conn.send_datagram(frame_datagram(15, false)).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        host_conn.send_datagram(frame_datagram(16, true)).unwrap();
        host_conn.send_datagram(frame_datagram(17, false)).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        host_conn.close(0u32.into(), b"test done");
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), recv_task).await;

        let decoded_ids = decoder.decoded_ids.lock().unwrap().clone();
        assert!(
            !decoded_ids.contains(&15) && decoded_ids.ends_with(&[16, 17]),
            "nothing between the backlog and the next keyframe may be decoded: {decoded_ids:?}"
        );
        let recorded = signals.lock().unwrap().clone();
        assert!(
            recorded
                .iter()
                .any(|s| matches!(s, RecoverySignal::DecodeBacklog)),
            "the backlog must ask for a keyframe: {recorded:?}"
        );
    }

    /// Over a real QUIC connection that loses 5% of fragments, answering the
    /// loop's retransmit requests must get every frame to the decoder, in
    /// order, without a single keyframe request.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_lossy_stream_is_recovered_by_retransmits_alone() {
        const FRAMES: u64 = 120;
        let (host_conn, viewer_conn) = loopback_pair().await;

        let mut receiver = DatagramReceiver::new();
        let decoder = RecordingDecoder::default();
        let mut decoder_handle = decoder.clone();
        let frame_queue = Arc::new(FrameQueue::new(8));
        let (loss_tx, mut loss_rx) = tokio::sync::mpsc::channel::<RecoverySignal>(1024);

        let recv_task = tokio::spawn(async move {
            let _ = receiver
                .run_receive_loop_with_loss_signal(
                    &viewer_conn,
                    &mut decoder_handle,
                    &frame_queue,
                    Some(loss_tx),
                )
                .await;
        });

        let sent: Arc<std::sync::Mutex<std::collections::HashMap<u64, Vec<Bytes>>>> =
            Arc::default();
        let keyframe_requests = Arc::new(AtomicU64::new(0));

        // The host side: answer every request from what was sent.
        let answer_conn = host_conn.clone();
        let answer_sent = Arc::clone(&sent);
        let answer_kf = Arc::clone(&keyframe_requests);
        tokio::spawn(async move {
            while let Some(signal) = loss_rx.recv().await {
                match signal {
                    RecoverySignal::Nack(requests) => {
                        let sent = answer_sent.lock().unwrap().clone();
                        for request in requests {
                            let Some(frags) = sent.get(&request.frame_id) else {
                                continue;
                            };
                            let ids: Vec<usize> = if request.frag_ids.is_empty() {
                                (0..frags.len()).collect()
                            } else {
                                request.frag_ids.iter().map(|&i| usize::from(i)).collect()
                            };
                            for i in ids {
                                let _ = answer_conn.send_datagram(frags[i].clone());
                            }
                        }
                    }
                    RecoverySignal::KeyframeNeeded | RecoverySignal::DecodeBacklog => {
                        answer_kf.fetch_add(1, Ordering::Relaxed);
                    }
                    _ => {}
                }
            }
        });

        // Deterministic 5% loss on first transmission.
        let mut rng = 0x2545_F491_4F6C_DD1Du64;
        for id in 1..=FRAMES {
            let frags = frame_fragments(id, id == 1, 3);
            sent.lock().unwrap().insert(id, frags.clone());
            for frag in frags {
                rng ^= rng << 13;
                rng ^= rng >> 7;
                rng ^= rng << 17;
                if rng % 100 >= 5 {
                    host_conn.send_datagram(frag).unwrap();
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(4)).await;
        }
        // A tail-loss probe, as the real sender sends once the stream goes quiet.
        let last = sent.lock().unwrap()[&FRAMES][2].clone();
        host_conn.send_datagram(last).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;

        host_conn.close(0u32.into(), b"test done");
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), recv_task).await;

        let decoded_ids = decoder.decoded_ids.lock().unwrap().clone();
        assert_eq!(
            decoded_ids,
            (1..=FRAMES).collect::<Vec<_>>(),
            "every frame, in order"
        );
        assert_eq!(keyframe_requests.load(Ordering::Relaxed), 0);
    }

    /// A lost fragment is asked for again, and the frames behind it wait for
    /// the retransmit instead of being decoded against a missing reference.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_lost_fragment_is_nacked_and_frames_wait_for_it() {
        let (host_conn, viewer_conn) = loopback_pair().await;

        let mut receiver = DatagramReceiver::new();
        let decoder = RecordingDecoder::default();
        let mut decoder_handle = decoder.clone();
        let frame_queue = Arc::new(FrameQueue::new(8));
        let (loss_tx, mut loss_rx) = tokio::sync::mpsc::channel::<RecoverySignal>(64);

        let recv_task = tokio::spawn(async move {
            let _ = receiver
                .run_receive_loop_with_loss_signal(
                    &viewer_conn,
                    &mut decoder_handle,
                    &frame_queue,
                    Some(loss_tx),
                )
                .await;
        });

        host_conn.send_datagram(frame_datagram(1, true)).unwrap();
        // Frame 2 loses its middle fragment; frame 3 follows intact.
        let frame2 = frame_fragments(2, false, 3);
        host_conn.send_datagram(frame2[0].clone()).unwrap();
        host_conn.send_datagram(frame2[2].clone()).unwrap();
        host_conn.send_datagram(frame_datagram(3, false)).unwrap();

        // The host answers the request it receives.
        let nack = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                match loss_rx.recv().await {
                    Some(RecoverySignal::Nack(requests)) => break requests,
                    Some(_) => {}
                    None => panic!("signal channel closed"),
                }
            }
        })
        .await
        .expect("a retransmit request");
        assert_eq!(nack.len(), 1);
        assert_eq!(nack[0].frame_id, 2);
        assert_eq!(nack[0].frag_ids, vec![1]);
        assert_eq!(*decoder.decoded_ids.lock().unwrap(), vec![1]);

        host_conn.send_datagram(frame2[1].clone()).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        host_conn.close(0u32.into(), b"test done");
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), recv_task).await;

        assert_eq!(*decoder.decoded_ids.lock().unwrap(), vec![1, 2, 3]);
    }
}
