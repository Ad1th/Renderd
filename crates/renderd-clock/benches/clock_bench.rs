#![allow(missing_docs)]
//! Micro-benchmarks for monotonic clock, epoch estimation, and rolling stats.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use renderd_clock::{ClockEpochEstimator, ClockSample, RollingStats};

fn bench_rolling_stats(c: &mut Criterion) {
    c.bench_function("rolling_stats_push_and_percentile", |b| {
        let mut stats = RollingStats::<60>::new();
        b.iter(|| {
            for i in 0..60 {
                stats.push(black_box(i * 100));
            }
            let p95 = stats.percentile(95.0);
            let mean = stats.mean();
            black_box((p95, mean));
        });
    });
}

fn bench_clock_estimator(c: &mut Criterion) {
    c.bench_function("clock_estimator_min_rtt_10_samples", |b| {
        b.iter(|| {
            let mut estimator = ClockEpochEstimator::new(10);
            for i in 0..10 {
                estimator.add_sample(black_box(ClockSample {
                    t1_ns: 100_000_000 + i * 10_000_000,
                    t2_ns: 150_000_000 + i * 10_000_000,
                    t3_ns: 155_000_000 + i * 10_000_000,
                    t4_ns: 205_000_000 + i * 10_000_000,
                }));
            }
            let est = estimator.estimate();
            black_box(est);
        });
    });
}

criterion_group!(benches, bench_rolling_stats, bench_clock_estimator);
criterion_main!(benches);
