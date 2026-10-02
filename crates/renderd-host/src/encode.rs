//! Video encoding dispatch pipeline for `renderd-host`.
//!
//! Directs hardware-accelerated video encoding via `VideoToolbox` (`renderd-vt-sys`) on macOS
//! and outputs encoded NAL units into a bounded lock-free ring buffer consumed by the
//! datagram sender.

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use crossbeam_channel::{bounded, Receiver, Sender};

use crate::error::HostError;
use crate::network::LinkPressure;

/// Number of encoded frames the ring buffer holds before the newest is dropped.
///
/// The sender drains this in microseconds per frame, so the buffer only fills if the
/// sender thread is descheduled; eight frames is ~130 ms of slack at 60 fps.
pub const RING_CAPACITY: usize = 8;

/// Shortest spacing between two keyframes the pipeline will produce on request.
pub const MIN_KEYFRAME_SPACING: Duration = Duration::from_millis(200);

/// Longest a keyframe request is ever deferred, however slow the link.
pub const MAX_KEYFRAME_SPACING: Duration = Duration::from_millis(1_500);

/// Sentinel for "no keyframe issued yet" in [`KeyframeGate::last_issued_us`].
const NEVER: u64 = u64::MAX;

/// [`MIN_KEYFRAME_SPACING`] in microseconds, the gate's clock unit.
const MIN_KEYFRAME_SPACING_US: u64 = 200_000;

/// Paces keyframe requests to what the link can actually carry.
///
/// Keyframe requests arrive from several places at once — the viewer after loss,
/// the ABR loop on entering Panic, the sender after skipping frames, the encoder
/// callback after a ring overflow — and each used to turn straight into an IDR.
/// On a fast LAN that is harmless. On a 6 Mbps link a mid-stream 1080p IDR is
/// ~100 KB, about 130 ms of link time, and the first one of a session is closer
/// to 400 KB: a second IDR requested while the first is still draining lands in
/// the same queue, delays every frame behind it, and the resulting lateness
/// looks like more loss, which requests more keyframes.
///
/// The gate keeps requests pending until the previous keyframe has had time to
/// drain — twice its own transmit time at the current bitrate, clamped to
/// [`MIN_KEYFRAME_SPACING`]..=[`MAX_KEYFRAME_SPACING`] — and then honours them
/// all with a single IDR. A request is never lost, only deferred.
#[derive(Debug)]
pub struct KeyframeGate {
    epoch: Instant,
    pending: AtomicBool,
    /// When the last keyframe was issued, in microseconds since `epoch`.
    last_issued_us: AtomicU64,
    /// Encoded size of the last keyframe, used to size the next cooldown.
    last_size_bytes: AtomicU64,
}

impl Default for KeyframeGate {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyframeGate {
    /// Creates a gate with no request pending and no keyframe history.
    #[must_use]
    pub fn new() -> Self {
        Self {
            epoch: Instant::now(),
            pending: AtomicBool::new(false),
            last_issued_us: AtomicU64::new(NEVER),
            last_size_bytes: AtomicU64::new(0),
        }
    }

    /// Asks for a keyframe as soon as the link allows one.
    pub fn request(&self) {
        self.pending.store(true, Ordering::SeqCst);
    }

    /// Forgets keyframe history and arms a request, so a new session's first
    /// frame is an IDR with no cooldown in its way.
    pub fn reset(&self) {
        self.last_issued_us.store(NEVER, Ordering::SeqCst);
        self.last_size_bytes.store(0, Ordering::SeqCst);
        self.pending.store(true, Ordering::SeqCst);
    }

    /// Returns `true` if a request is pending, whether or not it may fire yet.
    #[must_use]
    pub fn is_pending(&self) -> bool {
        self.pending.load(Ordering::SeqCst)
    }

    /// Returns `true` if the frame being encoded now should be a keyframe, and
    /// consumes the pending request if so.
    pub fn take(&self, bitrate_kbps: u32) -> bool {
        self.take_at(Instant::now(), bitrate_kbps)
    }

    /// [`Self::take`] at an explicit instant, for deterministic tests.
    pub fn take_at(&self, now: Instant, bitrate_kbps: u32) -> bool {
        if !self.pending.load(Ordering::SeqCst) {
            return false;
        }
        let now_us = self.micros_since_epoch(now);
        let last = self.last_issued_us.load(Ordering::SeqCst);
        if last != NEVER {
            let cooldown =
                Self::cooldown(self.last_size_bytes.load(Ordering::SeqCst), bitrate_kbps);
            let cooldown_us = u64::try_from(cooldown.as_micros()).unwrap_or(u64::MAX);
            if now_us.saturating_sub(last) < cooldown_us {
                return false;
            }
        }
        if !self.pending.swap(false, Ordering::SeqCst) {
            return false;
        }
        // Stamp the issue time now rather than when the encoder hands the IDR
        // back, so a second request in the few milliseconds of encode latency
        // cannot slip through the gate behind it.
        self.last_issued_us.store(now_us, Ordering::SeqCst);
        true
    }

    /// Records the encoded size of a keyframe that just came out of the encoder.
    ///
    /// Keyframes the encoder inserts on its own (the periodic GOP boundary) are
    /// recorded too, so they also hold back an immediately following request.
    pub fn record_keyframe(&self, size_bytes: usize) {
        self.record_keyframe_at(Instant::now(), size_bytes);
    }

    /// [`Self::record_keyframe`] at an explicit instant, for deterministic tests.
    pub fn record_keyframe_at(&self, now: Instant, size_bytes: usize) {
        self.last_size_bytes
            .store(size_bytes as u64, Ordering::SeqCst);
        let now_us = self.micros_since_epoch(now);
        let last = self.last_issued_us.load(Ordering::SeqCst);
        // An IDR the gate itself issued was already stamped in `take_at`; only
        // move the stamp forward for keyframes it did not know about.
        if last == NEVER || now_us.saturating_sub(last) > MIN_KEYFRAME_SPACING_US {
            self.last_issued_us.store(now_us, Ordering::SeqCst);
        }
    }

    /// Minimum spacing after a keyframe of `size_bytes` at `bitrate_kbps`: twice
    /// its transmit time, so the link has drained it and carried a few frames
    /// after it before the next one is queued.
    #[must_use]
    pub fn cooldown(size_bytes: u64, bitrate_kbps: u32) -> Duration {
        let kbps = u64::from(bitrate_kbps.max(1));
        // bytes * 8 bits / kbps = milliseconds of link time; doubled.
        let ms = size_bytes.saturating_mul(16) / kbps;
        Duration::from_millis(ms).clamp(MIN_KEYFRAME_SPACING, MAX_KEYFRAME_SPACING)
    }

    fn micros_since_epoch(&self, now: Instant) -> u64 {
        u64::try_from(now.saturating_duration_since(self.epoch).as_micros()).unwrap_or(NEVER - 1)
    }
}

/// How the encoder is set up, kept so it can be rebuilt at another size.
#[derive(Debug, Clone, PartialEq, Eq)]
struct EncoderParams {
    codec: String,
    width: u32,
    height: u32,
    frame_rate: u32,
}

/// What became of a surface handed to [`EncodePipeline::submit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Submitted {
    /// The surface went to the encoder.
    Encoded,
    /// The send queue was too deep; the surface was not encoded.
    Skipped,
}

/// Encoded video frame payload emitted by the hardware encoder into the ring buffer.
#[derive(Debug, Clone)]
pub struct EncodedFrame {
    /// Monotonically increasing frame sequence identifier.
    pub frame_id: u64,
    /// `true` if this frame is an IDR keyframe.
    pub is_keyframe: bool,
    /// Encoded H.264 / H.265 NAL unit byte payload.
    pub data: Bytes,
    /// Presentation timestamp in nanoseconds.
    pub pts_ns: i64,
}

/// Hardware video encoding dispatch pipeline.
///
/// Encapsulates encoder lifecycle, dynamic bitrate adjustment, force-keyframe trigger,
/// and ring-buffer distribution.
///
/// # Dropped frames
///
/// Every non-key frame references the frame before it. If a frame is dropped anywhere
/// between encoder and decoder, every subsequent frame decodes against the wrong
/// reference and the picture smears until the next keyframe. So whenever this
/// pipeline has to drop an encoded frame it also calls [`force_keyframe`], and the
/// [`KeyframeGate`] turns that into an IDR as soon as the link has room for one.
///
/// [`force_keyframe`]: EncodePipeline::force_keyframe
pub struct EncodePipeline {
    tx: Sender<EncodedFrame>,
    rx: Receiver<EncodedFrame>,
    frame_counter: AtomicU64,
    /// Frames the hardware encoder has emitted, which is also the source of
    /// their `frame_id`s. Never reset: a new encoder session for the same
    /// viewer must continue the ids, or the viewer discards everything it
    /// sends as stale.
    output_frames: Arc<AtomicU64>,
    keyframes: Arc<KeyframeGate>,
    dropped_frames: Arc<AtomicU64>,
    encoder_skipped: Arc<AtomicU64>,
    current_bitrate_kbps: AtomicU32,
    link: Arc<LinkPressure>,
    /// Presentation timestamp of the last surface submitted, in nanoseconds.
    last_pts_ns: AtomicI64,
    params: std::sync::Mutex<Option<EncoderParams>>,
    #[cfg(target_os = "macos")]
    session: std::sync::Mutex<Option<renderd_vt_sys::CompressionSession>>,
}

impl std::fmt::Debug for EncodePipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncodePipeline")
            .field("frame_counter", &self.frame_counter)
            .field("keyframes", &self.keyframes)
            .field("dropped_frames", &self.dropped_frames)
            .field("current_bitrate_kbps", &self.current_bitrate_kbps)
            .finish_non_exhaustive()
    }
}

impl Default for EncodePipeline {
    fn default() -> Self {
        Self::new()
    }
}

impl EncodePipeline {
    /// Creates a new `EncodePipeline` with a bounded ring buffer of [`RING_CAPACITY`] frames.
    #[must_use]
    pub fn new() -> Self {
        let (tx, rx) = bounded(RING_CAPACITY);
        Self {
            tx,
            rx,
            frame_counter: AtomicU64::new(1),
            output_frames: Arc::new(AtomicU64::new(0)),
            keyframes: Arc::new(KeyframeGate::new()),
            dropped_frames: Arc::new(AtomicU64::new(0)),
            encoder_skipped: Arc::new(AtomicU64::new(0)),
            current_bitrate_kbps: AtomicU32::new(0),
            link: Arc::new(LinkPressure::new()),
            last_pts_ns: AtomicI64::new(i64::MIN),
            params: std::sync::Mutex::new(None),
            #[cfg(target_os = "macos")]
            session: std::sync::Mutex::new(None),
        }
    }

    /// `pts_ns`, raised if needed to stay strictly after the last surface
    /// submitted.
    ///
    /// Capture timestamps and the re-encode passes of [`crate::refine`] come
    /// from different clocks; the encoder must never see time go backwards.
    pub fn monotonic_pts(&self, pts_ns: i64) -> i64 {
        let mut last = self.last_pts_ns.load(Ordering::Relaxed);
        loop {
            let next = pts_ns.max(last.saturating_add(1_000));
            match self.last_pts_ns.compare_exchange_weak(
                last,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return next,
                Err(seen) => last = seen,
            }
        }
    }

    /// Initializes the hardware compression session on macOS for the given resolution,
    /// bitrate, negotiated codec, and expected frame rate.
    ///
    /// `codec` is the string agreed during the Stream 0 handshake — `"h264"` or `"hevc"`.
    /// Anything else is rejected rather than silently encoding a stream the viewer said
    /// it cannot decode.
    ///
    /// # Errors
    ///
    /// Returns [`HostError::Initialization`] if the codec is unsupported or hardware
    /// encoder allocation fails.
    pub fn init(
        &self,
        width: u32,
        height: u32,
        bitrate_kbps: u32,
        codec: &str,
        frame_rate: u32,
    ) -> Result<(), HostError> {
        let codec_lower = codec.to_ascii_lowercase();
        if codec_lower != "h264" && codec_lower != "hevc" {
            return Err(HostError::Initialization(format!(
                "Unsupported codec '{codec}'; expected 'h264' or 'hevc'"
            )));
        }

        // Discard anything a previous session left queued so the new viewer's first
        // frame is this session's keyframe, not a stale P-frame from the last one.
        while self.rx.try_recv().is_ok() {}
        self.keyframes.reset();

        let params = EncoderParams {
            codec: codec_lower,
            width,
            height,
            frame_rate: frame_rate.max(1),
        };
        self.start_session(&params, bitrate_kbps)?;
        self.current_bitrate_kbps
            .store(bitrate_kbps, Ordering::Relaxed);
        tracing::info!(
            codec = %params.codec,
            width,
            height,
            bitrate_kbps,
            frame_rate,
            "Encoder configured"
        );
        if let Ok(mut guard) = self.params.lock() {
            *guard = Some(params);
        }
        Ok(())
    }

    /// Re-creates the encoder at `width` × `height` mid-session, keeping its
    /// codec, frame rate and current bitrate.
    ///
    /// The first frame of the new size is a keyframe, and frame ids carry on
    /// from the old encoder, so the viewer's decoder simply switches size at
    /// that keyframe. Frames the old encoder still had in flight are delivered
    /// first. Capture is left alone: `VideoToolbox` scales a captured surface
    /// to the session's size by itself.
    ///
    /// Returns `false`, doing nothing, if the encoder is already that size.
    ///
    /// # Errors
    ///
    /// Returns [`HostError::Initialization`] if the encoder was never
    /// configured or the new session cannot be created; the old one keeps
    /// running in that case.
    pub fn reconfigure(&self, width: u32, height: u32) -> Result<bool, HostError> {
        let mut params = self
            .params
            .lock()
            .map_err(|_| HostError::Initialization("EncodePipeline mutex poisoned".into()))?
            .clone()
            .ok_or_else(|| HostError::Initialization("encoder was never configured".into()))?;
        if (params.width, params.height) == (width, height) {
            return Ok(false);
        }
        params.width = width;
        params.height = height;
        self.keyframes.reset();
        self.start_session(&params, self.current_bitrate().max(1))?;
        tracing::info!(width, height, "Encoder resized to follow the link");
        if let Ok(mut guard) = self.params.lock() {
            *guard = Some(params);
        }
        Ok(true)
    }

    /// The size the encoder is producing, once configured.
    #[must_use]
    pub fn encoded_size(&self) -> Option<(u32, u32)> {
        self.params
            .lock()
            .ok()?
            .as_ref()
            .map(|p| (p.width, p.height))
    }

    /// Creates a hardware session for `params` and swaps it in. Dropping the
    /// old session waits for its in-flight frames to come out first.
    fn start_session(&self, params: &EncoderParams, bitrate_kbps: u32) -> Result<(), HostError> {
        #[cfg(target_os = "macos")]
        {
            use renderd_vt_sys::{CompressionSession, VideoCodec};

            // Encode what the viewer actually negotiated. Hardcoding HEVC here meant a
            // viewer that could only decode H.264 was sent a stream it could never show.
            let vt_codec = if params.codec == "h264" {
                VideoCodec::H264
            } else {
                VideoCodec::Hevc
            };

            let width_i32 = i32::try_from(params.width).map_err(|_| {
                HostError::Initialization("Width exceeds i32 max bounds".to_string())
            })?;
            let height_i32 = i32::try_from(params.height).map_err(|_| {
                HostError::Initialization("Height exceeds i32 max bounds".to_string())
            })?;

            let session = CompressionSession::with_frame_rate(
                width_i32,
                height_i32,
                vt_codec,
                bitrate_kbps,
                params.frame_rate,
                self.output_handler(),
            )
            .map_err(|e| {
                HostError::Initialization(format!("VTCompressionSession init failed: {e}"))
            })?;

            log_rate_controller(&session, &params.codec);

            let mut guard = self
                .session
                .lock()
                .map_err(|_| HostError::Initialization("EncodePipeline mutex poisoned".into()))?;
            // Drain the old session before the new one encodes anything. Frame ids
            // are handed out as frames come out of the encoder, so a last frame of
            // the old size finishing after the new size's first keyframe would get
            // the later id, and the viewer would decode it against the wrong
            // picture. Dropping waits for its in-flight frames — a few
            // milliseconds, during which capture waits on this lock.
            drop(guard.take());
            *guard = Some(session);
            drop(guard);
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (params, bitrate_kbps);
        }
        Ok(())
    }

    /// Builds the `VideoToolbox` output callback that feeds encoded frames into
    /// the ring buffer.
    #[cfg(target_os = "macos")]
    fn output_handler(
        &self,
    ) -> impl Fn(
        renderd_vt_sys::VtError,
        renderd_vt_sys::bindings::VTEncodeInfoFlags,
        renderd_vt_sys::bindings::CMSampleBufferRef,
    ) + Send
           + Sync
           + 'static {
        let tx = self.tx.clone();
        let keyframes = Arc::clone(&self.keyframes);
        let dropped = Arc::clone(&self.dropped_frames);
        let encoder_skipped = Arc::clone(&self.encoder_skipped);
        let count_atomic = Arc::clone(&self.output_frames);

        #[allow(unsafe_code)]
        move |err, flags, sample_buf| {
            if err.code() != 0 || sample_buf.is_null() {
                if err.code() == 0 && flags & renderd_vt_sys::ENCODE_INFO_FRAME_DROPPED != 0 {
                    // The low-latency rate controller had no budget for this
                    // frame. The encoder keeps its reference chain intact, so
                    // this is a lower frame rate, not a break in the stream.
                    encoder_skipped.fetch_add(1, Ordering::Relaxed);
                } else if err.code() != 0 {
                    tracing::warn!(
                        status = err.code(),
                        "VideoToolbox encode callback reported an error"
                    );
                }
                return;
            }
            // SAFETY: sample_buf is a valid CMSampleBufferRef delivered by VideoToolbox encoder.
            let Ok((nal_bytes, is_kf)) =
                (unsafe { renderd_vt_sys::sample_buffer_extract_nals(sample_buf) })
            else {
                return;
            };
            if nal_bytes.is_empty() {
                return;
            }
            let frame_id = count_atomic.fetch_add(1, Ordering::Relaxed) + 1;
            // Recover the capture timestamp VideoToolbox carried through the
            // encode. Without this every frame ships pts_ns = 0 and the viewer
            // has no presentation timing at all.
            // SAFETY: sample_buf was checked non-null above and is a valid
            // CMSampleBufferRef owned by the VideoToolbox callback.
            let pts_ns = unsafe { renderd_vt_sys::sample_buffer_presentation_time_ns(sample_buf) }
                .unwrap_or(0);
            if frame_id <= 3 || is_kf {
                tracing::info!(
                    frame_id,
                    is_keyframe = is_kf,
                    pts_ns,
                    data_len = nal_bytes.len(),
                    "Host Encoder: extracted VideoToolbox NAL units"
                );
            }

            if is_kf {
                keyframes.record_keyframe(nal_bytes.len());
            }

            let frame = EncodedFrame {
                frame_id,
                is_keyframe: is_kf,
                data: Bytes::from(nal_bytes),
                pts_ns,
            };
            if tx.try_send(frame).is_err() {
                // The sender is behind. Dropping this frame breaks the reference
                // chain, so resynchronise with an IDR rather than shipping
                // P-frames the decoder will render as smear.
                dropped.fetch_add(1, Ordering::Relaxed);
                keyframes.request();
            }
        }
    }

    /// Releases the hardware encoder session, if any.
    pub fn shutdown(&self) {
        self.link.detach();
        if let Ok(mut params) = self.params.lock() {
            *params = None;
        }
        #[cfg(target_os = "macos")]
        if let Ok(mut guard) = self.session.lock() {
            *guard = None;
        }
        while self.rx.try_recv().is_ok() {}
    }

    /// Submits a GPU `IoSurface` to the hardware encoder.
    ///
    /// Encoded output NAL units will be pushed to the ring buffer.
    ///
    /// # Errors
    ///
    /// Returns [`HostError::Initialization`] if hardware encoding fails.
    #[cfg(target_os = "macos")]
    pub fn encode_surface(
        &self,
        surface: &renderd_vt_sys::IoSurface,
        pts_ns: i64,
    ) -> Result<(), HostError> {
        self.submit(surface, pts_ns).map(|_| ())
    }

    /// Submits a GPU `IoSurface` to the hardware encoder, reporting whether it
    /// was encoded or skipped because the send queue is deep.
    ///
    /// # Errors
    ///
    /// Returns [`HostError::Initialization`] if hardware encoding fails.
    #[cfg(target_os = "macos")]
    pub fn submit(
        &self,
        surface: &renderd_vt_sys::IoSurface,
        pts_ns: i64,
    ) -> Result<Submitted, HostError> {
        // Shed a deep send queue here, before encoding, where skipping a frame
        // costs nothing: the encoder just sees a lower frame rate. A pending
        // keyframe request stays pending until a frame is actually encoded.
        if self.link.should_skip_frame(self.current_bitrate()) {
            return Ok(Submitted::Skipped);
        }
        let pts_ns = self.monotonic_pts(pts_ns);

        let force_kf = self.keyframes.take(self.current_bitrate());
        let frame_id = self.frame_counter.fetch_add(1, Ordering::SeqCst);

        let guard = self
            .session
            .lock()
            .map_err(|_| HostError::Initialization("EncodePipeline mutex poisoned".into()))?;

        if let Some(ref session) = *guard {
            let res = session
                .encode_surface(surface, pts_ns, force_kf)
                .map_err(|e| {
                    HostError::Initialization(format!("VideoToolbox encode_surface failed: {e}"))
                });
            drop(guard);
            if res.is_err() && force_kf {
                // The IDR never reached the encoder; keep the request alive.
                self.keyframes.request();
            }
            res?;
        } else {
            drop(guard);
            // Fallback / mock encoding path when session is not initialized
            let frame = EncodedFrame {
                frame_id,
                is_keyframe: force_kf || frame_id == 1,
                data: Bytes::from(vec![0u8; 128]),
                pts_ns,
            };
            let _ = self.tx.try_send(frame);
        }

        Ok(Submitted::Encoded)
    }

    /// Submits a raw byte payload to the encoding pipeline (used in mock / headless environments).
    ///
    /// If the ring buffer is full the frame is dropped, counted in
    /// [`EncodePipeline::dropped_frames`], and a keyframe is forced for the next frame.
    ///
    /// # Errors
    ///
    /// Never returns an error; the `Result` is retained for API compatibility.
    pub fn push_frame(&self, data: Bytes, pts_ns: i64) -> Result<(), HostError> {
        let force_kf = self.keyframes.take(self.current_bitrate());
        let frame_id = self.frame_counter.fetch_add(1, Ordering::SeqCst);
        let is_keyframe = force_kf || frame_id == 1;
        if is_keyframe {
            self.keyframes.record_keyframe(data.len());
        }

        let frame = EncodedFrame {
            frame_id,
            is_keyframe,
            data,
            pts_ns,
        };

        if self.tx.try_send(frame).is_err() {
            self.dropped_frames.fetch_add(1, Ordering::Relaxed);
            self.keyframes.request();
        }
        Ok(())
    }

    /// Requests an IDR keyframe as soon as the link has room for one.
    ///
    /// Usually that is the next encoded frame; right after another keyframe it is
    /// deferred until that one has drained (see [`KeyframeGate`]).
    pub fn force_keyframe(&self) {
        self.keyframes.request();
    }

    /// The send-queue backpressure shared with the session's network sender.
    #[must_use]
    pub const fn link(&self) -> &Arc<LinkPressure> {
        &self.link
    }

    /// Returns `true` if a keyframe request is waiting on the [`KeyframeGate`].
    #[must_use]
    pub fn keyframe_pending(&self) -> bool {
        self.keyframes.is_pending()
    }

    /// Dynamically updates the target bitrate in kilobits per second.
    ///
    /// Re-applying an unchanged bitrate is a no-op: every `VTSessionSetProperty` call
    /// on the rate-control keys resets the encoder's rate model, so calling it on each
    /// 100 ms telemetry tick — as the ABR loop does — visibly pumped quality.
    ///
    /// # Errors
    ///
    /// Returns [`HostError::Initialization`] if updating the hardware session property fails.
    pub fn set_bitrate(&self, bitrate_kbps: u32) -> Result<(), HostError> {
        if self
            .current_bitrate_kbps
            .swap(bitrate_kbps, Ordering::Relaxed)
            == bitrate_kbps
        {
            return Ok(());
        }

        #[cfg(target_os = "macos")]
        {
            let guard = self
                .session
                .lock()
                .map_err(|_| HostError::Initialization("EncodePipeline mutex poisoned".into()))?;
            if let Some(ref session) = *guard {
                session.set_bitrate(bitrate_kbps).map_err(|e| {
                    HostError::Initialization(format!(
                        "VTCompressionSession set_bitrate failed: {e}"
                    ))
                })?;
            }
        }

        tracing::info!(bitrate_kbps, "Encoder bitrate updated");
        Ok(())
    }

    /// Returns the bitrate most recently applied to the encoder, in kbps.
    #[must_use]
    pub fn current_bitrate(&self) -> u32 {
        self.current_bitrate_kbps.load(Ordering::Relaxed)
    }

    /// Returns a clone of the ring buffer output receiver.
    #[must_use]
    pub fn receiver(&self) -> Receiver<EncodedFrame> {
        self.rx.clone()
    }

    /// Frames the hardware encoder has emitted so far.
    #[must_use]
    pub fn frames_output(&self) -> u64 {
        self.output_frames.load(Ordering::Relaxed)
    }

    /// Returns the number of frames the encoder itself skipped for lack of bit budget.
    #[must_use]
    pub fn encoder_skipped_frames(&self) -> u64 {
        self.encoder_skipped.load(Ordering::Relaxed)
    }

    /// Returns the number of encoded frames dropped because the ring buffer was full.
    #[must_use]
    pub fn dropped_frames(&self) -> u64 {
        self.dropped_frames.load(Ordering::Relaxed)
    }
}

/// Reports which `VideoToolbox` rate controller `session` ended up with.
///
/// A session that silently fell back to the default controller behaves very
/// differently on a slow link, so that case is logged at WARN.
#[cfg(target_os = "macos")]
fn log_rate_controller(session: &renderd_vt_sys::CompressionSession, codec: &str) {
    if session.is_low_latency() {
        tracing::info!(codec, "VideoToolbox low-latency rate control enabled");
    } else {
        tracing::warn!(
            codec,
            "VideoToolbox refused low-latency rate control; frame sizes are only \
             bounded per second, so expect bursts on slow links"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_pipeline_ring_buffer_capacity() {
        let pipeline = EncodePipeline::new();
        let receiver = pipeline.receiver();

        for i in 0..RING_CAPACITY {
            let pts = i64::try_from(i).unwrap() * 16_000_000;
            pipeline
                .push_frame(Bytes::from_static(b"test"), pts)
                .unwrap();
        }

        // One more push drops gracefully due to the bounded capacity...
        pipeline
            .push_frame(Bytes::from_static(b"overflow"), 999)
            .unwrap();
        assert_eq!(pipeline.dropped_frames(), 1);

        for i in 0..RING_CAPACITY {
            let frame = receiver.try_recv().expect("frame in ring buffer");
            assert_eq!(frame.pts_ns, i64::try_from(i).unwrap() * 16_000_000);
        }
        assert!(receiver.try_recv().is_err());

        // ...and the drop leaves a keyframe request pending so the decoder can
        // resynchronise once the gate allows it.
        assert!(pipeline.keyframe_pending());
    }

    #[test]
    fn test_reconfigure_needs_a_configured_encoder_and_a_new_size() {
        let pipeline = EncodePipeline::new();
        assert!(pipeline.reconfigure(960, 544).is_err());
        if pipeline.init(1920, 1080, 4_000, "hevc", 60).is_err() {
            return; // no hardware encoder here
        }
        assert_eq!(pipeline.encoded_size(), Some((1920, 1080)));
        assert!(!pipeline.reconfigure(1920, 1080).unwrap());
        assert!(pipeline.reconfigure(960, 544).unwrap());
        assert_eq!(pipeline.encoded_size(), Some((960, 544)));
        pipeline.shutdown();
        assert_eq!(pipeline.encoded_size(), None);
    }

    #[test]
    fn test_monotonic_pts_never_goes_backwards() {
        let pipeline = EncodePipeline::new();
        assert_eq!(pipeline.monotonic_pts(5_000_000), 5_000_000);
        assert_eq!(pipeline.monotonic_pts(9_000_000), 9_000_000);
        // A capture timestamp older than a re-encode pass already submitted.
        assert_eq!(pipeline.monotonic_pts(8_000_000), 9_001_000);
    }

    #[test]
    fn test_min_spacing_constants_agree() {
        assert_eq!(
            u128::from(MIN_KEYFRAME_SPACING_US),
            MIN_KEYFRAME_SPACING.as_micros()
        );
    }

    #[test]
    fn test_gate_fires_immediately_without_history() {
        let gate = KeyframeGate::new();
        assert!(!gate.take(6_000), "nothing requested yet");
        gate.request();
        assert!(gate.take(6_000));
        assert!(!gate.is_pending(), "a taken request is consumed");
    }

    #[test]
    fn test_gate_defers_a_request_until_the_last_keyframe_drained() {
        let gate = KeyframeGate::new();
        let t0 = Instant::now();
        gate.request();
        assert!(gate.take_at(t0, 6_000));
        // 100 KB at 6 Mbps is ~133 ms of link time; the gate waits twice that.
        gate.record_keyframe_at(t0 + Duration::from_millis(5), 100_000);

        gate.request();
        assert!(!gate.take_at(t0 + Duration::from_millis(100), 6_000));
        assert!(!gate.take_at(t0 + Duration::from_millis(250), 6_000));
        assert!(gate.is_pending(), "a deferred request is never dropped");
        assert!(gate.take_at(t0 + Duration::from_millis(270), 6_000));
    }

    #[test]
    fn test_gate_coalesces_a_burst_of_requests_into_one_keyframe() {
        let gate = KeyframeGate::new();
        let t0 = Instant::now();
        gate.request();
        assert!(gate.take_at(t0, 6_000));
        gate.record_keyframe_at(t0, 100_000);
        for ms in (10..260).step_by(10) {
            gate.request();
            assert!(!gate.take_at(t0 + Duration::from_millis(ms), 6_000));
        }
        assert!(gate.take_at(t0 + Duration::from_millis(300), 6_000));
        assert!(!gate.take_at(t0 + Duration::from_millis(310), 6_000));
    }

    #[test]
    fn test_gate_counts_encoder_inserted_keyframes() {
        let gate = KeyframeGate::new();
        let t0 = Instant::now();
        // A periodic GOP keyframe the gate did not issue.
        gate.record_keyframe_at(t0, 100_000);
        gate.request();
        assert!(!gate.take_at(t0 + Duration::from_millis(50), 6_000));
    }

    #[test]
    fn test_gate_reset_clears_the_cooldown() {
        let gate = KeyframeGate::new();
        let t0 = Instant::now();
        gate.request();
        assert!(gate.take_at(t0, 6_000));
        gate.record_keyframe_at(t0, 400_000);
        gate.reset();
        assert!(gate.take_at(t0 + Duration::from_millis(1), 6_000));
    }

    #[test]
    fn test_gate_cooldown_scales_with_link_and_is_clamped() {
        assert_eq!(KeyframeGate::cooldown(100_000, 6_000).as_millis(), 266);
        assert_eq!(
            KeyframeGate::cooldown(100_000, 60_000),
            MIN_KEYFRAME_SPACING
        );
        assert_eq!(KeyframeGate::cooldown(400_000, 1_500), MAX_KEYFRAME_SPACING);
        assert_eq!(KeyframeGate::cooldown(100_000, 0), MAX_KEYFRAME_SPACING);
    }

    #[test]
    fn test_init_rejects_unknown_codec() {
        let pipeline = EncodePipeline::new();
        let err = pipeline.init(1920, 1080, 20_000, "vp9", 60).unwrap_err();
        assert!(
            format!("{err}").contains("vp9"),
            "error should name the codec: {err}"
        );
    }

    #[test]
    fn test_init_accepts_both_negotiable_codecs() {
        // On a machine without a usable hardware encoder these may still fail at the
        // VideoToolbox call; what must not happen is a rejection at the codec check.
        for codec in ["h264", "hevc", "HEVC", "H264"] {
            let pipeline = EncodePipeline::new();
            if let Err(e) = pipeline.init(640, 480, 4_000, codec, 60) {
                assert!(
                    !format!("{e}").contains("Unsupported codec"),
                    "{codec} must be accepted as a negotiable codec"
                );
            }
        }
    }

    #[test]
    fn test_force_keyframe_flag() {
        let pipeline = EncodePipeline::new();
        let receiver = pipeline.receiver();

        pipeline.force_keyframe();
        pipeline
            .push_frame(Bytes::from_static(b"frame2"), 32_000_000)
            .unwrap();

        let frame = receiver.try_recv().unwrap();
        assert!(frame.is_keyframe);
    }

    #[test]
    fn test_set_bitrate_is_idempotent_for_unchanged_value() {
        let pipeline = EncodePipeline::new();
        pipeline.set_bitrate(20_000).unwrap();
        pipeline.set_bitrate(20_000).unwrap();
        assert_eq!(pipeline.current_bitrate(), 20_000);
        pipeline.set_bitrate(25_000).unwrap();
        assert_eq!(pipeline.current_bitrate(), 25_000);
    }
}
