//! One-way queuing-delay estimator (`renderd-viewer/src/abr/delay.rs`).
//!
//! Every frame carries the host's capture timestamp. Its *transit* — local
//! arrival time minus that timestamp — mixes three things: the (unknown, fixed)
//! offset between the two machines' clocks, the fixed propagation and
//! processing time of the path, and whatever time the frame spent waiting in a
//! queue. The first two are constant, so the smallest transit seen recently is
//! the path with no queue at all, and anything above it is queuing delay. No
//! clock synchronisation is needed; the base window is short enough that
//! crystal drift between the machines never matters.
//!
//! Per report the estimator uses the *smallest* transit of the interval, not
//! the latest or the mean: a large keyframe arriving late because of its own
//! size inflates one sample, while a standing queue inflates every sample.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// How far back the no-queue baseline looks.
const BASE_WINDOW: Duration = Duration::from_secs(10);

/// Granularity of the baseline's sliding minimum.
const BUCKET: Duration = Duration::from_secs(1);

/// Reports in a row with no frame that repeat the last queue estimate before
/// it is dropped to zero.
///
/// A queue so deep that nothing arrives for a whole report is the worst case,
/// not the best one, so an empty interval must not read as "no queue". But a
/// still desktop legitimately sends nothing for seconds, so the estimate only
/// holds briefly.
const HOLD_EMPTY_REPORTS: u32 = 3;

/// One report's worth of delay and rate measurements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DelayReport {
    /// Queuing delay above the recent no-queue baseline, in microseconds.
    pub queue_delay_us: u32,
    /// RFC 3550 inter-arrival jitter, in microseconds.
    pub jitter_us: u32,
    /// Video payload received over the interval, in kbps.
    pub receive_rate_kbps: u32,
}

/// Tracks frame transit times and turns them into queuing delay, jitter and
/// receive rate.
#[derive(Debug)]
pub struct DelayTracker {
    epoch: Instant,
    /// `(bucket index, minimum transit in that bucket)`, oldest first.
    base_buckets: VecDeque<(u64, i64)>,
    interval_min_transit: Option<i64>,
    interval_bytes: u64,
    last_transit: Option<i64>,
    jitter_us: f64,
    last_queue_delay_us: u32,
    empty_reports: u32,
}

impl Default for DelayTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl DelayTracker {
    /// Creates an empty tracker.
    #[must_use]
    pub fn new() -> Self {
        Self::with_epoch(Instant::now())
    }

    /// Creates an empty tracker measuring local time from `epoch`.
    #[must_use]
    pub const fn with_epoch(epoch: Instant) -> Self {
        Self {
            epoch,
            base_buckets: VecDeque::new(),
            interval_min_transit: None,
            interval_bytes: 0,
            last_transit: None,
            jitter_us: 0.0,
            last_queue_delay_us: 0,
            empty_reports: 0,
        }
    }

    /// Records a frame whose host capture timestamp was `pts_ns`, `bytes` long,
    /// that finished arriving at `arrival`.
    #[allow(clippy::cast_precision_loss)]
    pub fn on_frame(&mut self, pts_ns: u64, bytes: usize, arrival: Instant) {
        let local_us = arrival.saturating_duration_since(self.epoch).as_micros();
        let local_us = i64::try_from(local_us).unwrap_or(i64::MAX);
        let captured_us = i64::try_from(pts_ns / 1_000).unwrap_or(i64::MAX);
        let transit = local_us.saturating_sub(captured_us);

        self.interval_bytes = self.interval_bytes.saturating_add(bytes as u64);
        self.interval_min_transit = Some(
            self.interval_min_transit
                .map_or(transit, |min| min.min(transit)),
        );

        // RFC 3550 §6.4.1: J += (|D| - J) / 16.
        if let Some(last) = self.last_transit {
            let d = transit.saturating_sub(last).unsigned_abs() as f64;
            self.jitter_us += (d - self.jitter_us) / 16.0;
        }
        self.last_transit = Some(transit);

        let bucket = u64::try_from(local_us.max(0)).unwrap_or(0)
            / u64::try_from(BUCKET.as_micros()).unwrap_or(1);
        match self.base_buckets.back_mut() {
            Some((idx, min)) if *idx == bucket => *min = (*min).min(transit),
            _ => self.base_buckets.push_back((bucket, transit)),
        }
        let window = BASE_WINDOW.as_secs() / BUCKET.as_secs().max(1);
        while self
            .base_buckets
            .front()
            .is_some_and(|(idx, _)| bucket.saturating_sub(*idx) >= window)
        {
            self.base_buckets.pop_front();
        }
    }

    /// Closes the current report interval of length `interval` and returns its
    /// measurements.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    pub fn take_report(&mut self, interval: Duration) -> DelayReport {
        let base = self.base_buckets.iter().map(|&(_, min)| min).min();
        let queue_delay_us =
            if let (Some(min), Some(base)) = (self.interval_min_transit.take(), base) {
                self.empty_reports = 0;
                u32::try_from(min.saturating_sub(base).max(0)).unwrap_or(u32::MAX)
            } else {
                self.empty_reports = self.empty_reports.saturating_add(1);
                if self.empty_reports <= HOLD_EMPTY_REPORTS {
                    self.last_queue_delay_us
                } else {
                    0
                }
            };
        self.last_queue_delay_us = queue_delay_us;

        let millis = interval.as_millis().max(1) as u64;
        let receive_rate_kbps =
            u32::try_from(self.interval_bytes.saturating_mul(8) / millis).unwrap_or(u32::MAX);
        self.interval_bytes = 0;

        DelayReport {
            queue_delay_us,
            jitter_us: self.jitter_us.round().clamp(0.0, f64::from(u32::MAX)) as u32,
            receive_rate_kbps,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: Duration = Duration::from_micros(16_667);
    const REPORT: Duration = Duration::from_millis(100);

    /// Host clock 5 s ahead of ours plus 3 ms of path: an arbitrary offset the
    /// estimator must cancel out.
    const HOST_OFFSET_NS: u64 = 5_000_000_000;

    struct Sim {
        tracker: DelayTracker,
        epoch: Instant,
        frame: u64,
    }

    impl Sim {
        fn new() -> Self {
            let epoch = Instant::now();
            Self {
                tracker: DelayTracker::with_epoch(epoch),
                epoch,
                frame: 0,
            }
        }

        /// Sends one frame captured now that spends `queue` in a queue.
        fn frame(&mut self, queue: Duration, bytes: usize) {
            let captured = FRAME * u32::try_from(self.frame).unwrap();
            let pts_ns = HOST_OFFSET_NS + u64::try_from(captured.as_nanos()).unwrap();
            let arrival = self.epoch + captured + Duration::from_millis(3) + queue;
            self.tracker.on_frame(pts_ns, bytes, arrival);
            self.frame += 1;
        }

        fn report_of(&mut self, frames: usize, queue: Duration) -> DelayReport {
            for _ in 0..frames {
                self.frame(queue, 12_500);
            }
            self.tracker.take_report(REPORT)
        }
    }

    #[test]
    fn test_no_queue_reads_zero_whatever_the_clock_offset() {
        let mut sim = Sim::new();
        for _ in 0..20 {
            let report = sim.report_of(6, Duration::ZERO);
            assert_eq!(report.queue_delay_us, 0);
        }
    }

    #[test]
    fn test_standing_queue_is_measured() {
        let mut sim = Sim::new();
        for _ in 0..10 {
            sim.report_of(6, Duration::ZERO);
        }
        let report = sim.report_of(6, Duration::from_millis(80));
        assert_eq!(report.queue_delay_us, 80_000);
    }

    /// One large, late frame (a keyframe waiting on its own bytes) must not
    /// read as a standing queue.
    #[test]
    fn test_single_late_frame_is_not_a_queue() {
        let mut sim = Sim::new();
        for _ in 0..10 {
            sim.report_of(6, Duration::ZERO);
        }
        sim.frame(Duration::from_millis(130), 100_000);
        let report = sim.report_of(5, Duration::ZERO);
        assert_eq!(report.queue_delay_us, 0);
    }

    #[test]
    fn test_receive_rate() {
        let mut sim = Sim::new();
        // 6 frames x 12.5 KB in 100 ms = 75 KB / 0.1 s = 6000 kbps.
        assert_eq!(sim.report_of(6, Duration::ZERO).receive_rate_kbps, 6_000);
        assert_eq!(sim.tracker.take_report(REPORT).receive_rate_kbps, 0);
    }

    #[test]
    fn test_empty_interval_holds_the_last_estimate_briefly() {
        let mut sim = Sim::new();
        for _ in 0..10 {
            sim.report_of(6, Duration::ZERO);
        }
        assert_eq!(
            sim.report_of(6, Duration::from_millis(90)).queue_delay_us,
            90_000
        );
        for _ in 0..HOLD_EMPTY_REPORTS {
            assert_eq!(sim.tracker.take_report(REPORT).queue_delay_us, 90_000);
        }
        assert_eq!(sim.tracker.take_report(REPORT).queue_delay_us, 0);
    }

    #[test]
    fn test_jitter_tracks_arrival_variation() {
        let mut sim = Sim::new();
        assert_eq!(sim.report_of(30, Duration::ZERO).jitter_us, 0);
        for i in 0..60 {
            let wobble = if i % 2 == 0 { 0 } else { 4 };
            sim.frame(Duration::from_millis(wobble), 10_000);
        }
        let jitter = sim.tracker.take_report(REPORT).jitter_us;
        assert!(
            (3_000..=4_000).contains(&jitter),
            "alternating 4 ms wobble should converge near 4 ms, got {jitter}"
        );
    }

    /// The baseline forgets old minima, so a route change that permanently
    /// lengthens the path stops reading as queue after the window passes.
    #[test]
    fn test_baseline_window_slides() {
        let mut sim = Sim::new();
        sim.report_of(6, Duration::ZERO);
        // Path is now 30 ms longer for good.
        let mut last = 0;
        for _ in 0..120 {
            last = sim.report_of(6, Duration::from_millis(30)).queue_delay_us;
        }
        assert_eq!(last, 0);
    }
}
