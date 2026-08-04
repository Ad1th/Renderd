#![allow(missing_docs)]
//! Micro-benchmarks for ABR engine state machine transitions and bitrate decisions.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use renderd_abr::AbrEngine;
use renderd_proto::types::BitrateKbps;

fn bench_abr_engine(c: &mut Criterion) {
    let mut engine = AbrEngine::new(
        BitrateKbps(5000),
        BitrateKbps(50000),
        BitrateKbps(15000),
        BitrateKbps(2000),
        0.02,
        0.10,
    );

    c.bench_function("abr_engine_update_steady", |b| {
        b.iter(|| {
            let decision = engine.update(black_box(0.01));
            black_box(decision);
        });
    });

    c.bench_function("abr_engine_update_panic", |b| {
        b.iter(|| {
            let decision = engine.update(black_box(0.15));
            black_box(decision);
        });
    });
}

criterion_group!(benches, bench_abr_engine);
criterion_main!(benches);
