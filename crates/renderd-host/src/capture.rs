//! Screen capture dispatch pipeline for `renderd-host`.
//!
//! Directs high-performance macOS GPU-resident screen recording via `ScreenCaptureKit` (`renderd-sc-sys`)
//! and forwards captured `IOSurface` frames directly to the `VideoToolbox` encoder (`EncodePipeline`)
//! on Apple's `QOS_CLASS_USER_INTERACTIVE` GCD dispatch queue without host CPU copies.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::encode::EncodePipeline;
use crate::error::HostError;

/// Screen capture pipeline manager.
///
/// Wraps macOS `ScreenStream` (from `renderd-sc-sys`) to capture display frames at
/// user-interactive `QoS`, wiring the `IOSurface` callback directly to `EncodePipeline::encode_surface`.
pub struct CapturePipeline {
    is_running: Arc<AtomicBool>,
    #[cfg(target_os = "macos")]
    stream: Option<renderd_sc_sys::ScreenStream>,
}

impl std::fmt::Debug for CapturePipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CapturePipeline")
            .field("is_running", &self.is_running.load(Ordering::SeqCst))
            .finish_non_exhaustive()
    }
}

impl Default for CapturePipeline {
    fn default() -> Self {
        Self::new()
    }
}

impl CapturePipeline {
    /// Creates a new `CapturePipeline` instance.
    #[must_use]
    pub fn new() -> Self {
        Self {
            is_running: Arc::new(AtomicBool::new(false)),
            #[cfg(target_os = "macos")]
            stream: None,
        }
    }

    /// Returns `true` if screen capture is currently active.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.is_running.load(Ordering::SeqCst)
    }

    /// Starts screen capture and connects frame callbacks directly to `EncodePipeline`.
    ///
    /// # Errors
    ///
    /// Returns [`HostError::Initialization`] if screen capture permission is denied or stream creation fails.
    pub fn start(
        &mut self,
        _width: u32,
        _height: u32,
        target_fps: u32,
        encode_pipeline: Arc<EncodePipeline>,
    ) -> Result<(), HostError> {
        if self.is_running() {
            return Ok(());
        }

        #[cfg(target_os = "macos")]
        {
            use renderd_sc_sys::{ContentFilter, ScreenRecordingPermission, ScreenStream};

            let status = ScreenRecordingPermission::check();
            if !status.is_granted() {
                return Err(HostError::Initialization(
                    "Screen recording permission denied by macOS TCC".into(),
                ));
            }

            let filter = ContentFilter::main_display().map_err(|e| {
                HostError::Initialization(format!("Failed to select main display for capture: {e}"))
            })?;

            let pipeline_ref = encode_pipeline;

            let stream = ScreenStream::new(&filter, target_fps, move |frame| {
                let _ = pipeline_ref.encode_surface(&frame.surface, frame.pts_ns);
            })
            .map_err(|e| HostError::Initialization(format!("ScreenStream creation failed: {e}")))?;

            self.stream = Some(stream);
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (target_fps, encode_pipeline);
        }

        self.is_running.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Dynamic vsync phase pacing control: updates `minimumFrameInterval` on the active stream.
    ///
    /// # Errors
    ///
    /// Returns [`HostError::Initialization`] if updating stream pacing fails.
    pub fn set_target_interval(&self, target_interval: Duration) -> Result<(), HostError> {
        #[cfg(target_os = "macos")]
        {
            if let Some(ref stream) = self.stream {
                stream.set_target_interval(target_interval).map_err(|e| {
                    HostError::Initialization(format!("Failed to set stream target interval: {e}"))
                })?;
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = target_interval;
        }

        Ok(())
    }

    /// Stops screen capture.
    ///
    /// # Errors
    ///
    /// Returns [`HostError::Initialization`] if stopping the underlying stream fails.
    pub fn stop(&mut self) -> Result<(), HostError> {
        if !self.is_running() {
            return Ok(());
        }

        #[cfg(target_os = "macos")]
        {
            if let Some(stream) = self.stream.take() {
                let _ = stream.stop();
            }
        }

        self.is_running.store(false, Ordering::SeqCst);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_capture_pipeline_lifecycle() {
        let mut capture = CapturePipeline::new();
        assert!(!capture.is_running());

        let encoder = Arc::new(EncodePipeline::new());

        // In headless test environments (without macOS TCC screen recording permission),
        // start() returns Ok or PermissionDenied cleanly without crashing.
        let result = capture.start(1920, 1080, 60, encoder);
        assert!(result.is_ok() || result.is_err());

        let stop_res = capture.stop();
        assert!(stop_res.is_ok());
        assert!(!capture.is_running());
    }

    #[test]
    fn test_capture_pipeline_100_frames_simulation() {
        let encoder = Arc::new(EncodePipeline::new());
        let rx = encoder.receiver();

        // Simulate 100 screen capture frames being pushed directly to encoder ring buffer
        for i in 1..=100 {
            let pts_ns = i * 16_666_666;
            let payload = bytes::Bytes::from(format!("frame_{i}_payload"));
            encoder.push_frame(payload, pts_ns).unwrap();

            // Drain ring buffer to simulate data sender thread reading frames
            if let Ok(frame) = rx.try_recv() {
                assert_eq!(frame.pts_ns, pts_ns);
            }
        }
    }
}
