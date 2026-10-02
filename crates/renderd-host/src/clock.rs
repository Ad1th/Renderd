//! Presentation clock synchronization controller integration for `renderd-host`.
//!
//! Receives [`VsyncReport`] messages from the viewer and adjusts capture pacing
//! on [`CapturePipeline`] to align host frame presentation with viewer vsync deadlines.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use renderd_clock::{ClockEpochEstimator, ClockSample};
use renderd_proto::generated::renderd::VsyncReport;

use crate::capture::CapturePipeline;
use crate::error::HostError;

/// Minimum time between two capture-rate reconfigurations.
///
/// Changing `minimumFrameInterval` makes `ScreenCaptureKit` rebuild its capture
/// pipeline, which drops a few frames and stalls the encoder. The viewer reports
/// vsync many times a second, so applying every report reconfigured capture
/// continuously — visible as constant stutter — and, after a few minutes, left
/// `SCStream` wedged and delivering nothing at all.
const MIN_RECONFIGURE_INTERVAL: Duration = Duration::from_secs(5);

/// Relative change in vsync period below which a report is treated as noise.
const PERIOD_DEADBAND: f64 = 0.01;

/// Host presentation clock sync manager.
///
/// Converts viewer vsync telemetry into capture pacing adjustments for `CapturePipeline`.
#[derive(Debug, Clone)]
pub struct ClockController {
    estimator: Arc<Mutex<ClockEpochEstimator>>,
    last_target_interval: Arc<Mutex<Duration>>,
    last_applied: Arc<Mutex<Option<(Duration, Instant)>>>,
    /// The negotiated frame interval: capture never runs faster than this.
    floor: Arc<Mutex<Duration>>,
}

impl Default for ClockController {
    fn default() -> Self {
        Self::new()
    }
}

impl ClockController {
    /// Creates a new `ClockController` initialized with default 60 Hz target interval (16.66 ms).
    #[must_use]
    pub fn new() -> Self {
        Self {
            estimator: Arc::new(Mutex::new(ClockEpochEstimator::new(16))),
            last_target_interval: Arc::new(Mutex::new(Duration::from_nanos(16_666_666))),
            last_applied: Arc::new(Mutex::new(None)),
            floor: Arc::new(Mutex::new(Duration::ZERO)),
        }
    }

    /// Starts a session whose capture was configured at `frame_interval`.
    ///
    /// Capture is already running at that interval, so it counts as applied:
    /// the first vsync report used to reconfigure `ScreenCaptureKit` to the
    /// interval it already had, rebuilding its pipeline — a stall and a few
    /// dropped frames — at the start of every session. The interval is also
    /// the floor for the session. The negotiated rate already accounts for the
    /// viewer's refresh rate and the host's `target_fps`, and a vsync report
    /// at 60 Hz used to lift a 30 fps session back to 60 fps — twice the
    /// frames for the bitrate the encoder was sized for.
    pub fn begin_session(&self, frame_interval: Duration) {
        if let Ok(mut floor) = self.floor.lock() {
            *floor = frame_interval;
        }
        if let Ok(mut applied) = self.last_applied.lock() {
            *applied = Some((frame_interval, Instant::now()));
        }
        if let Ok(mut last) = self.last_target_interval.lock() {
            *last = frame_interval;
        }
    }

    /// Processes a [`VsyncReport`] message from the connected viewer.
    ///
    /// Updates the presentation clock estimator every time. Returns the capture
    /// interval to apply when the reported period has really moved, at most
    /// once per [`MIN_RECONFIGURE_INTERVAL`], and never faster than the
    /// session's negotiated rate (see [`Self::begin_session`]).
    ///
    /// Applying it is left to the caller ([`apply_interval`]) because
    /// reconfiguring `ScreenCaptureKit` blocks until the capture pipeline is
    /// rebuilt, which must not happen on an async worker thread.
    ///
    /// # Errors
    /// Returns [`HostError::Initialization`] if an internal mutex is poisoned.
    pub fn on_vsync_report(&self, report: &VsyncReport) -> Result<Option<Duration>, HostError> {
        let vsync_period_ns = report.vsync_period_ns;

        // Default to 60 Hz (16,666,666 ns) if period is 0 or uninitialized
        let target_ns = if vsync_period_ns == 0 {
            16_666_666
        } else {
            vsync_period_ns.clamp(4_000_000, 33_333_333) // Clamp between 250 Hz and 30 Hz
        };

        let floor = *self
            .floor
            .lock()
            .map_err(|_| HostError::Initialization("ClockController mutex poisoned".into()))?;
        let target_interval = Duration::from_nanos(target_ns).max(floor);

        let sample = ClockSample {
            t1_ns: 0,
            t2_ns: report.vsync_phase_ns,
            t3_ns: report.vsync_phase_ns,
            t4_ns: 0,
        };

        let mut estimator = self
            .estimator
            .lock()
            .map_err(|_| HostError::Initialization("ClockController mutex poisoned".into()))?;
        estimator.add_sample(sample);
        drop(estimator);

        let mut last_guard = self
            .last_target_interval
            .lock()
            .map_err(|_| HostError::Initialization("ClockController mutex poisoned".into()))?;
        *last_guard = target_interval;
        drop(last_guard);

        if self.should_reconfigure(target_interval)? {
            tracing::info!(
                vsync_period_ns = report.vsync_period_ns,
                target_ms = target_interval.as_secs_f64() * 1000.0,
                "Retiming capture to the viewer's vsync period"
            );
            return Ok(Some(target_interval));
        }

        Ok(None)
    }

    /// Decides whether `target` differs enough from what was last applied, and enough
    /// time has passed, to justify reconfiguring the capture stream. Records the
    /// application when it returns `true`.
    fn should_reconfigure(&self, target: Duration) -> Result<bool, HostError> {
        let now = Instant::now();
        let mut applied = self
            .last_applied
            .lock()
            .map_err(|_| HostError::Initialization("ClockController mutex poisoned".into()))?;

        let apply = match *applied {
            None => true,
            Some((previous, at)) => {
                let delta = (target.as_secs_f64() - previous.as_secs_f64()).abs();
                let moved = delta > previous.as_secs_f64() * PERIOD_DEADBAND;
                moved && now.duration_since(at) >= MIN_RECONFIGURE_INTERVAL
            }
        };

        if apply {
            *applied = Some((target, now));
        }
        drop(applied);
        Ok(apply)
    }

    /// Returns the most recently computed target capture interval.
    ///
    /// # Panics
    /// Panics if the internal `Mutex` is poisoned.
    #[must_use]
    pub fn target_interval(&self) -> Duration {
        *self
            .last_target_interval
            .lock()
            .expect("ClockController mutex poisoned")
    }
}

/// Applies a capture interval [`ClockController::on_vsync_report`] asked for.
///
/// Blocks while `ScreenCaptureKit` rebuilds its pipeline; call it from a
/// blocking thread.
///
/// # Errors
/// Returns [`HostError::Initialization`] if the capture stream rejects it.
pub fn apply_interval(
    capture: &Mutex<CapturePipeline>,
    interval: Duration,
) -> Result<(), HostError> {
    capture
        .lock()
        .map_err(|_| HostError::Initialization("CapturePipeline mutex poisoned".into()))?
        .set_target_interval(interval)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(period_ns: u64) -> VsyncReport {
        VsyncReport {
            vsync_period_ns: period_ns,
            vsync_phase_ns: 1_000_000,
            clock_epoch_ns: 100_000_000,
        }
    }

    #[test]
    fn test_clock_controller_vsync_report_processing() {
        let controller = ClockController::new();
        let interval = controller.on_vsync_report(&report(16_666_666)).unwrap();
        assert_eq!(interval, Some(Duration::from_nanos(16_666_666)));
        assert_eq!(
            controller.target_interval(),
            Duration::from_nanos(16_666_666)
        );
    }

    /// Capture already runs at the negotiated rate; a report confirming it
    /// must not rebuild the capture pipeline.
    #[test]
    fn test_session_start_rate_counts_as_applied() {
        let controller = ClockController::new();
        controller.begin_session(Duration::from_nanos(16_666_666));
        assert_eq!(
            controller.on_vsync_report(&report(16_666_666)).unwrap(),
            None
        );
    }

    /// A 60 Hz viewer must not lift a 30 fps session to 60 fps.
    #[test]
    fn test_vsync_never_raises_capture_above_the_negotiated_rate() {
        let controller = ClockController::new();
        let thirty = Duration::from_nanos(33_333_333);
        controller.begin_session(thirty);
        assert_eq!(
            controller.on_vsync_report(&report(16_666_666)).unwrap(),
            None
        );
        assert_eq!(controller.target_interval(), thirty);
    }

    /// Repeated reports at the same period must not reconfigure capture again.
    #[test]
    fn test_reconfigure_is_throttled() {
        let controller = ClockController::new();
        let sixty = Duration::from_nanos(16_666_666);
        assert!(controller.should_reconfigure(sixty).unwrap());
        assert!(!controller.should_reconfigure(sixty).unwrap());
        // A real change, but inside the hold-off window: still suppressed.
        assert!(!controller
            .should_reconfigure(Duration::from_nanos(8_333_333))
            .unwrap());
    }
}
