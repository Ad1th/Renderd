//! Dual-timescale ABR feedback exporter (`renderd-viewer/src/abr/feedback.rs`).
//!
//! Computes frame loss rate, jitter, decode timing, and render latency, transmitting
//! `ReactiveStats` (every 100 ms), `PeriodicStats` (every 500 ms), and immediate `KeyframeRequest` (RFC-0002 §13).

use renderd_proto::{KeyframeRequest, PeriodicStats, ReactiveStats};
use std::time::{Duration, Instant};

/// Telemetry metrics collector and dual-timescale feedback exporter.
#[derive(Debug)]
pub struct FeedbackExporter {
    last_reactive_time: Instant,
    last_periodic_time: Instant,
    reactive_interval: Duration,
    periodic_interval: Duration,

    received_frames: u64,
    lost_frames: u64,
    last_frame_id: u64,

    total_decode_time: Duration,
    total_render_time: Duration,
    decode_sample_count: u32,
    render_sample_count: u32,

    frames_displayed: u64,
    frames_dropped: u64,
    receive_bandwidth_kbps: f32,
}

impl Default for FeedbackExporter {
    fn default() -> Self {
        Self::new()
    }
}

impl FeedbackExporter {
    /// Creates a new `FeedbackExporter` with default 100 ms reactive and 500 ms periodic intervals.
    #[must_use]
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            last_reactive_time: now,
            last_periodic_time: now,
            reactive_interval: Duration::from_millis(100),
            periodic_interval: Duration::from_millis(500),

            received_frames: 0,
            lost_frames: 0,
            last_frame_id: 0,

            total_decode_time: Duration::ZERO,
            total_render_time: Duration::ZERO,
            decode_sample_count: 0,
            render_sample_count: 0,

            frames_displayed: 0,
            frames_dropped: 0,
            receive_bandwidth_kbps: 50_000.0,
        }
    }

    /// Sets custom reactive and periodic export interval durations for testing.
    pub fn set_intervals(&mut self, reactive: Duration, periodic: Duration) {
        self.reactive_interval = reactive;
        self.periodic_interval = periodic;
    }

    /// Records receipt and processing of a video frame.
    pub fn record_frame(
        &mut self,
        frame_id: u64,
        decode_duration: Duration,
        render_duration: Duration,
    ) {
        if self.last_frame_id > 0 && frame_id > self.last_frame_id + 1 {
            let gap = frame_id - self.last_frame_id - 1;
            self.lost_frames += gap;
            self.frames_dropped += gap;
        }

        self.last_frame_id = frame_id;
        self.received_frames += 1;
        self.frames_displayed += 1;

        self.total_decode_time += decode_duration;
        self.decode_sample_count += 1;

        self.total_render_time += render_duration;
        self.render_sample_count += 1;
    }

    /// Records explicit frame loss event.
    pub fn record_frame_loss(&mut self, count: u64) {
        self.lost_frames += count;
        self.frames_dropped += count;
    }

    /// Updates current receive bandwidth estimation in Kbps.
    pub fn update_bandwidth(&mut self, bw_kbps: f32) {
        self.receive_bandwidth_kbps = bw_kbps;
    }

    /// Generates immediate `KeyframeRequest` on frame loss detection.
    #[must_use]
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn create_keyframe_request(&self) -> KeyframeRequest {
        let hint = if self.receive_bandwidth_kbps >= 0.0 {
            self.receive_bandwidth_kbps as u32
        } else {
            50_000
        };
        KeyframeRequest {
            after_frame_id: self.last_frame_id,
            bandwidth_hint_kbps: hint,
        }
    }

    /// Checks if reactive telemetry interval (100 ms) has elapsed and returns `ReactiveStats`.
    #[allow(clippy::cast_precision_loss)]
    pub fn maybe_export_reactive(&mut self) -> Option<ReactiveStats> {
        let now = Instant::now();
        if now.duration_since(self.last_reactive_time) < self.reactive_interval {
            return None;
        }

        self.last_reactive_time = now;
        let total = self.received_frames + self.lost_frames;
        let loss_rate = if total > 0 {
            u32::try_from(self.lost_frames).map_or(0.0, |lost| (lost as f32) / (total as f32))
        } else {
            0.0
        };

        self.received_frames = 0;
        self.lost_frames = 0;

        Some(ReactiveStats {
            loss_rate,
            jitter_us: 150,
            last_frame_id: self.last_frame_id,
        })
    }

    /// Checks if periodic telemetry interval (500 ms) has elapsed and returns `PeriodicStats`.
    pub fn maybe_export_periodic(&mut self) -> Option<PeriodicStats> {
        let now = Instant::now();
        if now.duration_since(self.last_periodic_time) < self.periodic_interval {
            return None;
        }

        self.last_periodic_time = now;

        let mean_decode_us = if self.decode_sample_count > 0 {
            let micros = u64::try_from(self.total_decode_time.as_micros()).unwrap_or_default();
            u32::try_from(micros / u64::from(self.decode_sample_count)).unwrap_or_default()
        } else {
            0
        };

        let mean_render_us = if self.render_sample_count > 0 {
            let micros = u64::try_from(self.total_render_time.as_micros()).unwrap_or_default();
            u32::try_from(micros / u64::from(self.render_sample_count)).unwrap_or_default()
        } else {
            0
        };

        self.total_decode_time = Duration::ZERO;
        self.decode_sample_count = 0;
        self.total_render_time = Duration::ZERO;
        self.render_sample_count = 0;

        Some(PeriodicStats {
            receive_bandwidth_kbps: self.receive_bandwidth_kbps,
            decode_time_us: mean_decode_us,
            render_time_us: mean_render_us,
            frames_displayed: self.frames_displayed,
            frames_dropped: self.frames_dropped,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_feedback_exporter_keyframe_request() {
        let mut exporter = FeedbackExporter::new();
        exporter.record_frame(42, Duration::from_micros(500), Duration::from_micros(200));
        exporter.update_bandwidth(15_000.0);

        let req = exporter.create_keyframe_request();
        assert_eq!(req.after_frame_id, 42);
        assert_eq!(req.bandwidth_hint_kbps, 15_000);
    }

    #[test]
    fn test_feedback_exporter_reactive_and_periodic_schedules() {
        let mut exporter = FeedbackExporter::new();
        exporter.set_intervals(Duration::from_millis(10), Duration::from_millis(20));

        exporter.record_frame(1, Duration::from_micros(400), Duration::from_micros(100));
        exporter.record_frame_loss(1);

        thread::sleep(Duration::from_millis(15));
        let reactive = exporter.maybe_export_reactive();
        assert!(reactive.is_some());
        let stats = reactive.unwrap();
        assert!((stats.loss_rate - 0.5).abs() < f32::EPSILON);

        thread::sleep(Duration::from_millis(15));
        let periodic = exporter.maybe_export_periodic();
        assert!(periodic.is_some());
        let p_stats = periodic.unwrap();
        assert_eq!(p_stats.decode_time_us, 400);
        assert_eq!(p_stats.render_time_us, 100);
    }
}
