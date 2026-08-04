//! Dual-timescale Adaptive Bitrate (ABR) control algorithm for Renderd.
//!
//! This crate implements the ABR controller described in RFC-0002 §13. It operates on
//! two separate feedback timescales — reactive (100 ms) and proactive (500 ms) — to
//! separate transient loss handling from steady-state bandwidth estimation.
//!
//! # Architecture
//!
//! The crate contains three modules:
//!
//! - [`engine`] — [`AbrEngine`]: the core decision engine that maps loss metrics to
//!   [`BitrateDecision`] outputs. Stateless with respect to I/O; driven by the caller.
//! - [`state`] — [`AbrState`]: the four-state FSM (`Steady` → `ProbeUp` ↔ `Backoff`
//!   → `Panic`). Transitions are deterministic pure functions, enabling property-based
//!   testing without side effects.
//! - [`telemetry`] — [`TelemetryReport`]: converts raw `PeriodicStats` /
//!   `ReactiveStats` proto messages (from `renderd-proto`) into normalized inputs
//!   for the engine.
//!
//! # Usage
//!
//! ```rust,no_run
//! use renderd_abr::{AbrEngine, AbrState};
//! use renderd_proto::types::BitrateKbps;
//!
//! // Construct engine with min / max / initial / step / thresholds
//! let mut engine = AbrEngine::new(
//!     BitrateKbps(5_000),   // min
//!     BitrateKbps(50_000),  // max
//!     BitrateKbps(20_000),  // initial (1080p60 default)
//!     BitrateKbps(2_000),   // step per interval
//!     0.05,                 // loss_threshold: 5% triggers Backoff
//!     0.20,                 // panic_threshold: 20% triggers Panic
//! );
//!
//! // Drive the engine from the control plane on each ReactiveStats receipt
//! let decision = engine.update(0.0 /* loss_rate */);
//! println!("target: {:?}, keyframe: {}", decision.target_bitrate_kbps, decision.request_keyframe);
//! ```
//!
//! # Design Note
//!
//! [`AbrEngine`] is a **pure state machine**: it does not spawn tasks, hold timers, or
//! perform I/O. The host application drives it by calling [`AbrEngine::update`] on
//! receipt of each feedback message. This makes the engine trivially testable with
//! deterministic input sequences.
//!
//! # Panics
//!
//! This crate does not panic. All operations return values or errors rather than panicking.

pub mod engine;
pub mod state;
pub mod telemetry;

pub use engine::*;
pub use state::*;
pub use telemetry::*;
