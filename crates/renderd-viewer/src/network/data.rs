//! Datagram receiver & sliding-window reassembly task (`renderd-viewer/src/network/data.rs`).
//!
//! Receives UDP/QUIC datagrams, parses 16-byte fragment headers, feeds the sliding-window
//! `ReassemblyBuffer`, and hands completed frame bitstreams to the video decoder (RFC-0002 §12.2).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use bytes::Bytes;
use futures_util::FutureExt as _;
use renderd_frame::{FragmentHeader, ReassembledFrame, ReassemblyBuffer, HEADER_SIZE};

use crate::decoder::Decoder;
use crate::error::ViewerError;
use crate::frame_queue::FrameQueue;

/// Upper bound on how many already-queued datagrams a single drain pass will pull
/// before yielding back to decode. Not a normal limit — only a safety valve against
/// an unbounded loop if a peer somehow floods the connection.
const DRAIN_CAP: usize = 4096;

/// What the receive loop is telling the control-plane feedback task.
///
/// The two cases must stay distinct: both want an immediate keyframe, but only one
/// of them is evidence the *network* is struggling. Conflating them (as a single
/// loss counter previously did) meant that every local, CPU-bound decode backlog —
/// which is common under sustained motion, exactly when scrolling stresses the
/// software decoder hardest — got reported to the host as packet loss and pulled
/// the encoder bitrate down for no network reason at all, visibly hurting quality
/// during scrolling on top of the separate latency problem it caused.
#[derive(Debug, Clone, Copy)]
pub enum RecoverySignal {
    /// Fragments were actually lost in transit or evicted from the reassembly
    /// window incomplete. Real evidence of network trouble — this should count
    /// toward the reported loss rate the host's ABR loop reacts to.
    FragmentLoss(u64),
    /// Decode fell behind datagram arrival and frames were skipped to catch up.
    /// A purely local, CPU-bound event; it asks for a keyframe but must never be
    /// reported as loss.
    DecodeBacklog,
}

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
        match self.reassemble(datagram)? {
            Some(frame) => {
                let pts_ns = u64::from(frame.pts_offset_us) * 1000;
                decoder.decode_packet(&frame.payload, frame.frame_id, pts_ns)?;
                Ok(Some(frame.frame_id))
            }
            None => Ok(None),
        }
    }

    /// Parses and reassembles one raw datagram, without decoding.
    ///
    /// Split out from [`Self::process_datagram`] so the receive loop can drain a
    /// backlog of already-queued datagrams — cheap memory copies — while reserving
    /// the expensive decode step for frames it actually intends to show.
    ///
    /// # Errors
    /// Returns [`ViewerError::Network`] if header parsing or reassembly fails.
    fn reassemble(&mut self, datagram: &[u8]) -> Result<Option<ReassembledFrame>, ViewerError> {
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

                Ok(Some(frame))
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

    /// Runs the datagram receiver event loop with an optional channel to signal frame loss for immediate keyframe requests.
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
    /// # Standing latency
    ///
    /// A synchronous software decode that ever runs even slightly slower than the
    /// incoming frame rate — true of Media Foundation's software H.264/HEVC path
    /// under sustained motion, where every frame differs a lot and compresses
    /// worse — used to leave every datagram queued inside `quinn`'s own receive
    /// buffer and decoded strictly in arrival order. The loop never caught up: it
    /// kept faithfully decoding older and older frames, and what the viewer showed
    /// fell further behind the live desktop the longer the stream ran. That is the
    /// multi-second lag this loop exists to prevent.
    ///
    /// Each pass now blocks for the next datagram, then immediately drains
    /// anything `quinn` already has queued with a non-blocking poll (no separate
    /// task or channel needed — `read_datagram()` returns a fresh future each
    /// call, so polling one once and discarding it if not ready costs nothing).
    /// A drain that finds more than one datagram means arrival has outrun
    /// consumption; every fragment still gets reassembled to keep the sliding
    /// window correct, but non-keyframe decodes are skipped until the next
    /// keyframe arrives — decoding a P-frame whose reference is already stale
    /// wastes CPU on a frame that only makes the picture worse. A keyframe request
    /// goes out immediately so that next frame arrives quickly.
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
        static REASM_FRAME_COUNT: AtomicU64 = AtomicU64::new(0);
        static DECODED_FRAME_COUNT: AtomicU64 = AtomicU64::new(0);
        static SKIPPED_STALE_COUNT: AtomicU64 = AtomicU64::new(0);

        let mut interval_start = std::time::Instant::now();
        let mut interval_datagrams: u64 = 0;
        let mut interval_reassembled: u64 = 0;
        let mut interval_bytes: u64 = 0;
        let mut interval_decoded: u64 = 0;
        let mut interval_skipped: u64 = 0;
        let mut interval_backlog_events: u64 = 0;
        let mut last_dropped_frames = self.window.dropped_frames();

        // Set whenever a backlog forces frames to be skipped; cleared the moment a
        // keyframe is actually decoded. While set, only keyframes are decoded.
        let mut awaiting_keyframe = false;

        loop {
            // Block for at least one datagram; this is the only await in the loop,
            // so the task sleeps entirely between bursts rather than polling.
            let Ok(first) = connection.read_datagram().await else {
                break;
            };

            let mut batch: Vec<Bytes> = Vec::with_capacity(4);
            batch.push(first);

            // Pull anything already sitting in quinn's receive queue without
            // waiting. `now_or_never` polls the freshly constructed future exactly
            // once and gives up instantly if nothing is ready — this is what lets
            // the loop catch up to a burst instead of draining it one datagram,
            // and one await, at a time.
            while batch.len() < DRAIN_CAP {
                match connection.read_datagram().now_or_never() {
                    Some(Ok(more)) => batch.push(more),
                    _ => break,
                }
            }

            let backlog = batch.len() > 1;
            if backlog {
                interval_backlog_events += 1;
                if !awaiting_keyframe {
                    awaiting_keyframe = true;
                    tracing::warn!(
                        queued = batch.len(),
                        "DatagramReceiver: arrival outran decode — skipping to the next keyframe"
                    );
                }
                if let Some(ref tx) = loss_tx {
                    let _ = tx.try_send(RecoverySignal::DecodeBacklog);
                }
            }

            for datagram in &batch {
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

                match self.reassemble(datagram) {
                    Ok(Some(frame)) => {
                        let frame_count = REASM_FRAME_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
                        interval_reassembled += 1;

                        if frame_count == 1 {
                            tracing::info!(
                                count = frame_count,
                                frame_id = frame.frame_id,
                                "DatagramReceiver: first frame reassembled"
                            );
                        }

                        if awaiting_keyframe && !frame.is_keyframe {
                            // Its reference frame was never decoded; decoding this
                            // one would only display corrupted motion. Drop it and
                            // keep waiting for the keyframe already requested above.
                            SKIPPED_STALE_COUNT.fetch_add(1, Ordering::Relaxed);
                            interval_skipped += 1;
                            continue;
                        }
                        awaiting_keyframe = false;

                        let pts_ns = u64::from(frame.pts_offset_us) * 1000;
                        if let Err(e) =
                            decoder.decode_packet(&frame.payload, frame.frame_id, pts_ns)
                        {
                            tracing::warn!("decode_packet failed for frame {}: {e}", frame.frame_id);
                            continue;
                        }

                        // Drain everything the decoder has ready, not just one frame.
                        let mut pushed_any = false;
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
                            pushed_any = true;
                        }
                        if pushed_any {
                            if let Some(wake) = on_frame {
                                wake();
                            }
                        }
                    }
                    Ok(None) => {}
                    Err(_) => {
                        if let Some(ref tx) = loss_tx {
                            let _ = tx.try_send(RecoverySignal::FragmentLoss(1));
                        }
                    }
                }
            }

            let current_drops = self.window.dropped_frames();
            if current_drops > last_dropped_frames {
                let diff = current_drops - last_dropped_frames;
                last_dropped_frames = current_drops;
                if let Some(ref tx) = loss_tx {
                    let _ = tx.try_send(RecoverySignal::FragmentLoss(diff));
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
                    skipped_stale = interval_skipped,
                    backlog_events = interval_backlog_events,
                    catching_up = awaiting_keyframe,
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
                interval_skipped = 0;
                interval_backlog_events = 0;
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

    /// Test-only decoder that records the id of every frame it is actually asked
    /// to decode, so a test can assert on *which* frames a backlog skipped.
    #[derive(Debug, Default, Clone)]
    struct RecordingDecoder {
        decoded_ids: std::sync::Arc<std::sync::Mutex<Vec<u64>>>,
        pending: std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<DecodedFrame>>>,
    }

    impl Decoder for RecordingDecoder {
        fn initialize(&mut self, _codec: &str, _width: u32, _height: u32) -> Result<(), ViewerError> {
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
        let cert_gen = rcgen::generate_simple_self_signed(vec!["renderd-test".to_string()]).unwrap();
        let cert_der = rustls::pki_types::CertificateDer::from(cert_gen.cert.der().to_vec());
        let key_der =
            rustls::pki_types::PrivateKeyDer::Pkcs8(cert_gen.key_pair.serialize_der().into());

        let server_tls = renderd_net::ServerTlsConfig::from_cert(vec![cert_der], key_der, None).unwrap();
        let client_tls = renderd_net::ClientTlsConfig::with_insecure_skip_verify().unwrap();

        let server = renderd_net::QuicServer::bind("127.0.0.1:0".parse().unwrap(), server_tls).unwrap();
        let addr = server.local_addr().unwrap();
        let client = renderd_net::QuicClient::bind_ephemeral().unwrap();

        let accept = tokio::spawn(async move { server.accept().await.unwrap() });
        let client_conn = client.connect(addr, "renderd-test", client_tls).await.unwrap();
        let server_conn = accept.await.unwrap();
        (server_conn, client_conn)
    }

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
            pts_offset_us: 0,
        };
        let mut buf = vec![0u8; HEADER_SIZE];
        header.encode(&mut buf).unwrap();
        buf.extend_from_slice(&[0xAB; 8]);
        Bytes::from(buf)
    }

    /// The regression test for the multi-second lag this module exists to fix:
    /// once arrival outruns consumption, the receive loop must skip non-keyframe
    /// backlog rather than faithfully decoding every stale frame in order.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_receive_loop_skips_stale_backlog_until_next_keyframe() {
        let (host_conn, viewer_conn) = loopback_pair().await;

        let mut receiver = DatagramReceiver::new(4);
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
        host_conn
            .send_datagram(frame_datagram(1, true))
            .unwrap();
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
            !recorded_signals.is_empty(),
            "a backlog occurred and must have signalled for a keyframe"
        );
        assert!(
            recorded_signals
                .iter()
                .all(|s| matches!(s, RecoverySignal::DecodeBacklog)),
            "no fragment was actually lost, so no signal may report FragmentLoss: {recorded_signals:?}"
        );
    }
}
