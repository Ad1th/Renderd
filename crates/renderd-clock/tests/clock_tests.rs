//! Integration tests for clock offset estimation and rolling statistics.

use std::time::Duration;

use renderd_clock::{ClockEpochEstimator, ClockSample, MonoInstant, RollingStats};

#[test]
fn test_epoch_estimator_min_rtt_selection() {
    let mut estimator = ClockEpochEstimator::new(10);

    // High RTT sample (RTT = 200ms, Offset = 0ms)
    estimator.add_sample(ClockSample {
        t1_ns: 100_000_000,
        t2_ns: 200_000_000,
        t3_ns: 210_000_000,
        t4_ns: 310_000_000,
    });

    // Low RTT sample (RTT = 20ms, Offset = +50ms)
    estimator.add_sample(ClockSample {
        t1_ns: 500_000_000,
        t2_ns: 560_000_000,
        t3_ns: 565_000_000,
        t4_ns: 525_000_000,
    });

    // High RTT jitter sample (RTT = 300ms, Offset = +50ms)
    estimator.add_sample(ClockSample {
        t1_ns: 900_000_000,
        t2_ns: 1_100_000_000,
        t3_ns: 1_110_000_000,
        t4_ns: 1_210_000_000,
    });

    let best = estimator.estimate().expect("Estimate should exist");
    assert_eq!(best.rtt_ns, 20_000_000);
    assert_eq!(best.offset_ns, 50_000_000);
}

#[test]
fn test_rolling_stats_ring_overwrite() {
    let mut stats = RollingStats::<3>::new();

    stats.push(100);
    stats.push(200);
    stats.push(300);
    assert_eq!(stats.len(), 3);
    assert_eq!(stats.min(), Some(100));

    // Overwrite oldest (100) with 400
    stats.push(400);
    assert_eq!(stats.len(), 3);
    assert_eq!(stats.min(), Some(200));
    assert_eq!(stats.max(), Some(400));
    assert!((stats.mean() - 300.0).abs() < 1e-6);
}

#[test]
fn test_mono_instant_ops() {
    let t0 = MonoInstant::now();
    let d = Duration::from_millis(50);
    let t1 = t0 + d;

    assert_eq!(t1 - t0, d);
    assert_eq!(t1.checked_duration_since(t0), Some(d));
    assert_eq!(t0.checked_duration_since(t1), None);
}
