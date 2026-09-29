//! Closed-loop simulation of the ABR engine against a bottleneck link.
//!
//! A fluid model: every 100 ms report the encoder produces `bitrate × 0.1 s`,
//! the link drains at most `capacity × 0.1 s`, and the remainder queues. The
//! viewer reports the queue as one-way delay and what it drained as the
//! receive rate — exactly the signals `update_signals` consumes. The engine
//! must keep the queue short (a responsive stream) while still using most of
//! the link (a sharp one), including across a sudden capacity drop.

// A numeric model, not production code: plain arithmetic reads better here.
#![allow(
    clippy::suboptimal_flops,
    clippy::missing_const_for_fn,
    clippy::cast_precision_loss
)]

use renderd_abr::{AbrEngine, Signals};
use renderd_proto::types::BitrateKbps;

const TICK_S: f64 = 0.1;

struct Link {
    queue_kbit: f64,
}

struct Tick {
    bitrate_kbps: f64,
    delay_ms: f64,
    received_kbps: f64,
}

impl Link {
    fn step(&mut self, bitrate_kbps: f64, capacity_kbps: f64) -> Tick {
        self.queue_kbit += bitrate_kbps * TICK_S;
        let drained = self.queue_kbit.min(capacity_kbps * TICK_S);
        self.queue_kbit -= drained;
        Tick {
            bitrate_kbps,
            delay_ms: self.queue_kbit / capacity_kbps * 1_000.0,
            received_kbps: drained / TICK_S,
        }
    }
}

fn engine(initial: u32) -> AbrEngine {
    AbrEngine::new(
        BitrateKbps(1_500),
        BitrateKbps(10_000),
        BitrateKbps(initial),
        BitrateKbps(1_000),
        0.02,
        0.15,
    )
}

/// Runs `ticks` reports against a link whose capacity is `capacity(tick)`.
fn run(engine: &mut AbrEngine, ticks: usize, capacity: impl Fn(usize) -> f64) -> Vec<Tick> {
    let mut link = Link { queue_kbit: 0.0 };
    let mut out = Vec::with_capacity(ticks);
    let mut signals = Signals::default();
    for t in 0..ticks {
        let decision = engine.update_signals(&signals);
        let tick = link.step(f64::from(decision.target_bitrate_kbps.0), capacity(t));
        signals = Signals {
            queue_delay_ms: tick.delay_ms,
            receive_rate_kbps: tick.received_kbps,
            ..Signals::default()
        };
        out.push(tick);
    }
    out
}

fn percentile(mut values: Vec<f64>, p: f64) -> f64 {
    values.sort_by(f64::total_cmp);
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let idx = ((values.len() - 1) as f64 * p).round() as usize;
    values[idx]
}

fn mean(values: impl Iterator<Item = f64>) -> f64 {
    let (sum, n) = values.fold((0.0, 0.0), |(s, n), v| (s + v, n + 1.0));
    sum / n
}

/// Starting well above a 6 Mbps link, the engine must shed the queue within a
/// couple of seconds and then hold it short while keeping the link busy.
#[test]
fn test_converges_below_a_6_mbps_bottleneck() {
    let mut engine = engine(10_000);
    let ticks = run(&mut engine, 600, |_| 6_000.0);

    let settle = 30; // 3 s
    let steady = &ticks[settle..];
    let p95_delay = percentile(steady.iter().map(|t| t.delay_ms).collect(), 0.95);
    let utilisation = mean(steady.iter().map(|t| t.received_kbps)) / 6_000.0;
    let peak = ticks.iter().map(|t| t.delay_ms).fold(0.0, f64::max);
    println!(
        "6 Mbps: peak delay {peak:.0} ms, steady p95 {p95_delay:.0} ms, utilisation {:.0}%",
        utilisation * 100.0
    );

    assert!(p95_delay < 60.0, "steady-state p95 queue {p95_delay:.0} ms");
    assert!(
        utilisation > 0.75,
        "link only {:.0}% used",
        utilisation * 100.0
    );
    assert!(peak < 700.0, "initial overshoot queued {peak:.0} ms");
}

/// The link halving mid-stream (someone starts a download) must be absorbed
/// quickly, not after seconds of stale video.
#[test]
fn test_recovers_from_a_capacity_drop() {
    let mut engine = engine(4_000);
    let drop_at = 200;
    let ticks = run(&mut engine, 600, |t| {
        if t < drop_at {
            6_000.0
        } else {
            3_000.0
        }
    });

    let after = &ticks[drop_at..];
    let recovered_in = after
        .iter()
        .enumerate()
        .skip(5)
        .find(|(i, _)| after[*i..].iter().take(20).all(|t| t.delay_ms < 100.0))
        .map(|(i, _)| i)
        .expect("queue never recovered after the drop");
    let peak = after.iter().map(|t| t.delay_ms).fold(0.0, f64::max);
    let utilisation = mean(after[50..].iter().map(|t| t.received_kbps)) / 3_000.0;
    println!(
        "6 -> 3 Mbps: peak delay {peak:.0} ms, queue back under 100 ms after {} ms, utilisation {:.0}%",
        recovered_in * 100,
        utilisation * 100.0
    );

    assert!(
        recovered_in <= 20,
        "took {} ms to drain",
        recovered_in * 100
    );
    assert!(peak < 500.0, "queue peaked at {peak:.0} ms");
    assert!(utilisation > 0.7);
}

/// After the congestion clears, the engine must find its way back up.
#[test]
fn test_climbs_back_when_capacity_returns() {
    let mut engine = engine(4_000);
    let ticks = run(&mut engine, 900, |t| {
        if (200..400).contains(&t) {
            3_000.0
        } else {
            8_000.0
        }
    });
    let late = mean(ticks[800..].iter().map(|t| t.bitrate_kbps));
    println!("capacity back to 8 Mbps: bitrate after 40 s {late:.0} kbps");
    assert!(late > 6_000.0, "only climbed back to {late:.0} kbps");
}

/// Loss-only operation (an old viewer that reports no delay) is unchanged in
/// kind: no delay signal, so the engine does not move on a clean link.
#[test]
fn test_bitrate_never_leaves_bounds_under_any_capacity() {
    let mut engine = engine(10_000);
    for tick in run(&mut engine, 2_000, |t| 500.0 + (t % 97) as f64 * 150.0) {
        assert!((1_500.0..=10_000.0).contains(&tick.bitrate_kbps));
    }
}
