//! Telemetry ingestion logic processing protobuf stats reports per RFC-0002 §14.3.

use renderd_proto::{PeriodicStats, ReactiveStats};

/// Processed telemetry report containing normalized loss rate and bandwidth.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TelemetryReport {
    /// Calculated loss rate between 0.0 and 1.0.
    pub loss_rate: f64,
    /// Measured receive bandwidth in Kbps (0.0 if unavailable).
    pub receive_bandwidth_kbps: f32,
}

impl TelemetryReport {
    /// Extracts a `TelemetryReport` from a `PeriodicStats` protobuf message.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn from_periodic(stats: &PeriodicStats) -> Option<Self> {
        let total = stats.frames_displayed.saturating_add(stats.frames_dropped);
        if total == 0 {
            return None;
        }

        let loss_rate = (stats.frames_dropped as f64 / total as f64).clamp(0.0, 1.0);

        Some(Self {
            loss_rate,
            receive_bandwidth_kbps: stats.receive_bandwidth_kbps,
        })
    }

    /// Extracts a `TelemetryReport` from a `ReactiveStats` protobuf message.
    #[must_use]
    pub fn from_reactive(stats: &ReactiveStats) -> Self {
        Self {
            loss_rate: f64::from(stats.loss_rate).clamp(0.0, 1.0),
            receive_bandwidth_kbps: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn test_periodic_stats_extraction() {
        let stats = PeriodicStats {
            receive_bandwidth_kbps: 25000.0,
            decode_time_us: 1500,
            render_time_us: 2000,
            frames_displayed: 98,
            frames_dropped: 2,
        };

        let report = TelemetryReport::from_periodic(&stats).unwrap();
        assert!((report.loss_rate - 0.02).abs() < 1e-6);
        assert!((report.receive_bandwidth_kbps - 25000.0).abs() < 1e-6);
    }

    #[test]
    fn test_reactive_stats_extraction() {
        let stats = ReactiveStats {
            loss_rate: 0.05,
            jitter_us: 1000,
            last_frame_id: 100,
        };

        let report = TelemetryReport::from_reactive(&stats);
        assert!((report.loss_rate - 0.05).abs() < 1e-6);
    }
}
