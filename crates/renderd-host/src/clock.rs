//! Presentation clock synchronization controller integration for `renderd-host`.
//!
//! Receives [`VsyncReport`] messages from the viewer and adjusts capture pacing
//! on [`CapturePipeline`] to align host frame presentation with viewer vsync deadlines.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use renderd_clock::{ClockEpochEstimator, ClockSample};
use renderd_proto::generated::renderd::VsyncReport;

use crate::capture::CapturePipeline;
use crate::error::HostError;

/// Host presentation clock sync manager.
///
/// Converts viewer vsync telemetry into capture pacing adjustments for `CapturePipeline`.
#[derive(Debug, Clone)]
pub struct ClockController {
    estimator: Arc<Mutex<ClockEpochEstimator>>,
    last_target_interval: Arc<Mutex<Duration>>,
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
        }
    }

    /// Processes a [`VsyncReport`] message from the connected viewer.
    ///
    /// Updates presentation clock sync estimator and adjusts stream capture target interval on `capture_pipeline`.
    ///
    /// # Errors
    /// Returns [`HostError::Initialization`] if updating target interval on `capture_pipeline` fails.
    pub fn on_vsync_report(
        &self,
        report: &VsyncReport,
        capture_pipeline: &CapturePipeline,
    ) -> Result<Duration, HostError> {
        let vsync_period_ns = report.vsync_period_ns;

        // Default to 60 Hz (16,666,666 ns) if period is 0 or uninitialized
        let target_ns = if vsync_period_ns == 0 {
            16_666_666
        } else {
            vsync_period_ns.clamp(4_000_000, 33_333_333) // Clamp between 250 Hz and 30 Hz
        };

        let target_interval = Duration::from_nanos(target_ns);

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

        capture_pipeline.set_target_interval(target_interval)?;

        tracing::debug!(
            vsync_period_ns = report.vsync_period_ns,
            target_ms = target_interval.as_secs_f64() * 1000.0,
            "Updated capture pipeline vsync phase pacing"
        );

        Ok(target_interval)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::CapturePipeline;

    #[test]
    fn test_clock_controller_vsync_report_processing() {
        let controller = ClockController::new();
        let capture = CapturePipeline::new();

        let report = VsyncReport {
            vsync_period_ns: 16_666_666, // 60 Hz
            vsync_phase_ns: 1_000_000,
            clock_epoch_ns: 100_000_000,
        };

        let interval = controller.on_vsync_report(&report, &capture).unwrap();
        assert_eq!(interval, Duration::from_nanos(16_666_666));
        assert_eq!(controller.target_interval(), interval);
    }
}
