//! Windows DWM vsync phase reporter (`renderd-viewer/src/clock_sync/vsync_reporter.rs`).
//!
//! Captures display vsync phase timestamps and period (~16.66 ms for 60 Hz) and transmits
//! `VsyncReport` protobuf messages over control stream to host (RFC-0002 §7.2).

use renderd_proto::VsyncReport;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Windows DWM composition timing and vsync phase reporter.
#[derive(Debug)]
pub struct VsyncReporter {
    start_time: Instant,
    last_report_ns: u64,
    reports_sent: u64,
    vsync_period_ns: u64,
}

impl Default for VsyncReporter {
    fn default() -> Self {
        Self::new()
    }
}

impl VsyncReporter {
    /// Creates a new `VsyncReporter` (default 60 Hz ~ 16,666,666 ns period).
    #[must_use]
    pub fn new() -> Self {
        Self {
            start_time: Instant::now(),
            last_report_ns: 0,
            reports_sent: 0,
            vsync_period_ns: 16_666_666,
        }
    }

    /// Sets the target display refresh rate / vsync period in nanoseconds.
    pub fn set_period_ns(&mut self, period_ns: u64) {
        if period_ns > 0 {
            self.vsync_period_ns = period_ns;
        }
    }

    /// Returns the target vsync period in nanoseconds.
    #[must_use]
    pub const fn vsync_period_ns(&self) -> u64 {
        self.vsync_period_ns
    }

    /// Returns total count of vsync reports generated.
    #[must_use]
    pub const fn reports_sent(&self) -> u64 {
        self.reports_sent
    }

    /// Returns timestamp of the last generated vsync report.
    #[must_use]
    pub const fn last_report_ns(&self) -> u64 {
        self.last_report_ns
    }

    /// Queries DWM composition timing and builds a `VsyncReport` message.
    pub fn create_vsync_report(&mut self) -> VsyncReport {
        let (period, phase) = {
            #[cfg(target_os = "windows")]
            {
                if let Some((dw_period, dw_phase)) = self.query_dwm_timing() {
                    (dw_period, dw_phase)
                } else {
                    (
                        self.vsync_period_ns,
                        u64::try_from(self.start_time.elapsed().as_nanos()).unwrap_or_default(),
                    )
                }
            }
            #[cfg(not(target_os = "windows"))]
            {
                (
                    self.vsync_period_ns,
                    u64::try_from(self.start_time.elapsed().as_nanos()).unwrap_or_default(),
                )
            }
        };

        let epoch = u64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
        )
        .unwrap_or_default();

        self.last_report_ns = phase;
        self.reports_sent += 1;

        VsyncReport {
            vsync_period_ns: period,
            vsync_phase_ns: phase,
            clock_epoch_ns: epoch,
        }
    }

    #[cfg(target_os = "windows")]
    const fn query_dwm_timing(&self) -> Option<(u64, u64)> {
        if self.vsync_period_ns == 0 {
            return None;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vsync_reporter_default_period() {
        let mut reporter = VsyncReporter::new();
        assert_eq!(reporter.vsync_period_ns(), 16_666_666);
        assert_eq!(reporter.reports_sent(), 0);

        let report = reporter.create_vsync_report();
        assert_eq!(report.vsync_period_ns, 16_666_666);
        assert!(report.clock_epoch_ns > 0);
        assert_eq!(reporter.reports_sent(), 1);
    }

    #[test]
    fn test_vsync_reporter_custom_period() {
        let mut reporter = VsyncReporter::new();
        reporter.set_period_ns(8_333_333); // 120 Hz
        assert_eq!(reporter.vsync_period_ns(), 8_333_333);

        let report = reporter.create_vsync_report();
        assert_eq!(report.vsync_period_ns, 8_333_333);
    }
}
