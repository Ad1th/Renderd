//! Video encoding dispatch pipeline for `renderd-host`.
//!
//! Directs hardware-accelerated video encoding via `VideoToolbox` (`renderd-vt-sys`) on macOS
//! and outputs encoded NAL units into a capacity-4 lock-free SPSC ring buffer.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use bytes::Bytes;
use crossbeam_channel::{bounded, Receiver, Sender};

use crate::error::HostError;

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
/// and lock-free SPSC ring-buffer distribution (capacity 4).
pub struct EncodePipeline {
    tx: Sender<EncodedFrame>,
    rx: Receiver<EncodedFrame>,
    frame_counter: AtomicU64,
    force_keyframe_flag: AtomicBool,
    dropped_frames: AtomicU64,
    #[cfg(target_os = "macos")]
    session: std::sync::Mutex<Option<renderd_vt_sys::CompressionSession>>,
}

impl std::fmt::Debug for EncodePipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncodePipeline")
            .field("frame_counter", &self.frame_counter)
            .field("force_keyframe_flag", &self.force_keyframe_flag)
            .field("dropped_frames", &self.dropped_frames)
            .finish_non_exhaustive()
    }
}

impl Default for EncodePipeline {
    fn default() -> Self {
        Self::new()
    }
}

impl EncodePipeline {
    /// Creates a new `EncodePipeline` with a capacity-4 lock-free ring buffer.
    #[must_use]
    pub fn new() -> Self {
        let (tx, rx) = bounded(4);
        Self {
            tx,
            rx,
            frame_counter: AtomicU64::new(1),
            force_keyframe_flag: AtomicBool::new(false),
            dropped_frames: AtomicU64::new(0),
            #[cfg(target_os = "macos")]
            session: std::sync::Mutex::new(None),
        }
    }

    /// Initializes the hardware compression session on macOS for the given resolution,
    /// bitrate, and negotiated codec.
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
    ) -> Result<(), HostError> {
        let codec_lower = codec.to_ascii_lowercase();
        if codec_lower != "h264" && codec_lower != "hevc" {
            return Err(HostError::Initialization(format!(
                "Unsupported codec '{codec}'; expected 'h264' or 'hevc'"
            )));
        }

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

            let width_i32 = i32::try_from(width).map_err(|_| {
                HostError::Initialization("Width exceeds i32 max bounds".to_string())
            })?;
            let height_i32 = i32::try_from(height).map_err(|_| {
                HostError::Initialization("Height exceeds i32 max bounds".to_string())
            })?;

            let count_atomic = std::sync::Arc::new(AtomicU64::new(0));

            let session = CompressionSession::new(
                width_i32,
                height_i32,
                vt_codec,
                bitrate_kbps,
                #[allow(unsafe_code)]
                move |err, _flags, sample_buf| {
                    if err.code() == 0 && !sample_buf.is_null() {
                        // SAFETY: sample_buf is a valid CMSampleBufferRef delivered by VideoToolbox encoder.
                        if let Ok((nal_bytes, is_kf)) = unsafe { renderd_vt_sys::sample_buffer_extract_nals(sample_buf) } {
                            if !nal_bytes.is_empty() {
                                let frame_id = count_atomic.fetch_add(1, Ordering::Relaxed) + 1;
                                // Recover the capture timestamp VideoToolbox carried through
                                // the encode. Without this every frame ships pts_ns = 0 and
                                // the viewer has no presentation timing at all.
                                // SAFETY: sample_buf was checked non-null above and is a valid
                                // CMSampleBufferRef owned by the VideoToolbox callback.
                                let pts_ns = unsafe {
                                    renderd_vt_sys::sample_buffer_presentation_time_ns(sample_buf)
                                }
                                .unwrap_or(0);
                                if frame_id <= 5 || is_kf {
                                    let first_32 = &nal_bytes[..32.min(nal_bytes.len())];
                                    tracing::info!(
                                        frame_id = frame_id,
                                        is_keyframe = is_kf,
                                        pts_ns = pts_ns,
                                        data_len = nal_bytes.len(),
                                        first_32_bytes = ?first_32,
                                        "Host Encoder: extracted real VideoToolbox NAL units from CMSampleBufferRef"
                                    );
                                }

                                let frame = EncodedFrame {
                                    frame_id,
                                    is_keyframe: is_kf,
                                    data: Bytes::from(nal_bytes),
                                    pts_ns,
                                };
                                let _ = tx.try_send(frame);
                            }
                        }
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
            let _ = (width, height, bitrate_kbps);
        }

        tracing::info!(codec = %codec_lower, width, height, bitrate_kbps, "Encoder configured");
        Ok(())
    }

    /// Submits a GPU `IoSurface` to the hardware encoder.
    ///
    /// Encoded output NAL units will be pushed to the lock-free ring buffer.
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
    /// If the capacity-4 ring buffer is full the frame is dropped and the drop is counted
    /// in [`EncodePipeline::dropped_frames`]; dropping the newest frame is the correct
    /// latency-preserving behaviour for a live stream, so this is not an error.
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

        // Try sending to SPSC ring buffer (capacity 4). If full, drop the frame.
        if self.tx.try_send(frame).is_err() {
            self.dropped_frames.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }

    /// Requests an immediate IDR keyframe for the next encoded frame.
    pub fn force_keyframe(&self) {
        self.force_keyframe_flag.store(true, Ordering::SeqCst);
    }

    /// Dynamically updates the target bitrate in Kilobits per second (Kbps).
    ///
    /// # Errors
    ///
    /// Returns [`HostError::Initialization`] if updating the hardware session property fails.
    pub fn set_bitrate(&self, bitrate_kbps: u32) -> Result<(), HostError> {
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

        #[cfg(not(target_os = "macos"))]
        {
            let _ = bitrate_kbps;
        }

        Ok(())
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
    fn test_encode_pipeline_ring_buffer_capacity_4() {
        let pipeline = EncodePipeline::new();
        let receiver = pipeline.receiver();

        // Push 4 frames (should all succeed)
        for i in 0..4 {
            pipeline
                .push_frame(Bytes::from_static(b"test"), i * 16_000_000)
                .unwrap();
        }

        // 5th frame push should drop gracefully due to bounded capacity 4
        pipeline
            .push_frame(Bytes::from_static(b"overflow"), 64_000_000)
            .unwrap();

        // Verify receiver receives first 4 frames
        for i in 0..4 {
            let frame = receiver.try_recv().expect("frame in ring buffer");
            assert_eq!(frame.pts_ns, i * 16_000_000);
        }

        // Ring buffer is now empty
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn test_init_rejects_unknown_codec() {
        let pipeline = EncodePipeline::new();
        let err = pipeline.init(1920, 1080, 20_000, "vp9").unwrap_err();
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
            if let Err(e) = pipeline.init(640, 480, 4_000, codec) {
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
}
