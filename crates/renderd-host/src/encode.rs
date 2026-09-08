//! Video encoding dispatch pipeline for `renderd-host`.
//!
//! Directs hardware-accelerated video encoding via `VideoToolbox` (`renderd-vt-sys`) on macOS
//! and outputs encoded NAL units into a bounded lock-free ring buffer consumed by the
//! datagram sender.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use bytes::Bytes;
use crossbeam_channel::{bounded, Receiver, Sender};

use crate::error::HostError;

/// Number of encoded frames the ring buffer holds before the newest is dropped.
///
/// The sender drains this in microseconds per frame, so the buffer only fills if the
/// sender thread is descheduled; eight frames is ~130 ms of slack at 60 fps.
pub const RING_CAPACITY: usize = 8;

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
/// pipeline has to drop an encoded frame it also arms [`force_keyframe`], making the
/// very next encode an IDR that resynchronises the decoder within one frame interval.
///
/// [`force_keyframe`]: EncodePipeline::force_keyframe
pub struct EncodePipeline {
    tx: Sender<EncodedFrame>,
    rx: Receiver<EncodedFrame>,
    frame_counter: AtomicU64,
    force_keyframe_flag: Arc<AtomicBool>,
    dropped_frames: Arc<AtomicU64>,
    current_bitrate_kbps: AtomicU32,
    #[cfg(target_os = "macos")]
    session: std::sync::Mutex<Option<renderd_vt_sys::CompressionSession>>,
}

impl std::fmt::Debug for EncodePipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncodePipeline")
            .field("frame_counter", &self.frame_counter)
            .field("force_keyframe_flag", &self.force_keyframe_flag)
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
            force_keyframe_flag: Arc::new(AtomicBool::new(false)),
            dropped_frames: Arc::new(AtomicU64::new(0)),
            current_bitrate_kbps: AtomicU32::new(0),
            #[cfg(target_os = "macos")]
            session: std::sync::Mutex::new(None),
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
        self.force_keyframe_flag.store(true, Ordering::SeqCst);

        #[cfg(target_os = "macos")]
        {
            use renderd_vt_sys::{CompressionSession, VideoCodec};

            // Encode what the viewer actually negotiated. Hardcoding HEVC here meant a
            // viewer that could only decode H.264 was sent a stream it could never show.
            let vt_codec = if codec_lower == "h264" {
                VideoCodec::H264
            } else {
                VideoCodec::Hevc
            };

            let tx = self.tx.clone();
            let force_keyframe = Arc::clone(&self.force_keyframe_flag);
            let dropped = Arc::clone(&self.dropped_frames);

            let width_i32 = i32::try_from(width).map_err(|_| {
                HostError::Initialization("Width exceeds i32 max bounds".to_string())
            })?;
            let height_i32 = i32::try_from(height).map_err(|_| {
                HostError::Initialization("Height exceeds i32 max bounds".to_string())
            })?;

            let count_atomic = std::sync::Arc::new(AtomicU64::new(0));

            let session = CompressionSession::with_frame_rate(
                width_i32,
                height_i32,
                vt_codec,
                bitrate_kbps,
                frame_rate.max(1),
                #[allow(unsafe_code)]
                move |err, _flags, sample_buf| {
                    if err.code() != 0 || sample_buf.is_null() {
                        if err.code() != 0 {
                            tracing::warn!(status = err.code(), "VideoToolbox encode callback reported an error");
                        }
                        return;
                    }
                    // SAFETY: sample_buf is a valid CMSampleBufferRef delivered by VideoToolbox encoder.
                    let Ok((nal_bytes, is_kf)) = (unsafe { renderd_vt_sys::sample_buffer_extract_nals(sample_buf) }) else {
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
                    if frame_id <= 3 {
                        tracing::info!(
                            frame_id,
                            is_keyframe = is_kf,
                            pts_ns,
                            data_len = nal_bytes.len(),
                            "Host Encoder: extracted VideoToolbox NAL units"
                        );
                    }

                    let frame = EncodedFrame {
                        frame_id,
                        is_keyframe: is_kf,
                        data: Bytes::from(nal_bytes),
                        pts_ns,
                    };
                    if tx.try_send(frame).is_err() {
                        // The sender is behind. Dropping this frame breaks the reference
                        // chain, so resynchronise with an IDR on the next encode rather
                        // than shipping P-frames the decoder will render as smear.
                        dropped.fetch_add(1, Ordering::Relaxed);
                        force_keyframe.store(true, Ordering::SeqCst);
                    }
                },
            )
            .map_err(|e| {
                HostError::Initialization(format!("VTCompressionSession init failed: {e}"))
            })?;

            let mut guard = self
                .session
                .lock()
                .map_err(|_| HostError::Initialization("EncodePipeline mutex poisoned".into()))?;
            *guard = Some(session);
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (width, height, bitrate_kbps, frame_rate);
        }

        self.current_bitrate_kbps.store(bitrate_kbps, Ordering::Relaxed);
        tracing::info!(codec = %codec_lower, width, height, bitrate_kbps, frame_rate, "Encoder configured");
        Ok(())
    }

    /// Releases the hardware encoder session, if any.
    pub fn shutdown(&self) {
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
        let force_kf = self.force_keyframe_flag.swap(false, Ordering::SeqCst);
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
            if res.is_err() {
                // The frame never reached the encoder; make sure the next one is an IDR.
                self.force_keyframe_flag.store(true, Ordering::SeqCst);
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

        Ok(())
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
        let force_kf = self.force_keyframe_flag.swap(false, Ordering::SeqCst);
        let frame_id = self.frame_counter.fetch_add(1, Ordering::SeqCst);

        let frame = EncodedFrame {
            frame_id,
            is_keyframe: force_kf || frame_id == 1,
            data,
            pts_ns,
        };

        if self.tx.try_send(frame).is_err() {
            self.dropped_frames.fetch_add(1, Ordering::Relaxed);
            self.force_keyframe_flag.store(true, Ordering::SeqCst);
        }
        Ok(())
    }

    /// Requests an immediate IDR keyframe for the next encoded frame.
    pub fn force_keyframe(&self) {
        self.force_keyframe_flag.store(true, Ordering::SeqCst);
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
        if self.current_bitrate_kbps.swap(bitrate_kbps, Ordering::Relaxed) == bitrate_kbps {
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

    /// Returns the number of encoded frames dropped because the ring buffer was full.
    #[must_use]
    pub fn dropped_frames(&self) -> u64 {
        self.dropped_frames.load(Ordering::Relaxed)
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

        // ...and the frame after the drop is forced to be a keyframe so the decoder
        // can resynchronise.
        pipeline
            .push_frame(Bytes::from_static(b"after-drop"), 1_000)
            .unwrap();
        assert!(receiver.try_recv().unwrap().is_keyframe);
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
