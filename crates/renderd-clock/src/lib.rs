//! High-resolution monotonic clock wrapper and NTP/PTP clock offset estimator.
//!
//! This crate implements the clock primitives needed for Renderd's presentation clock
//! synchronization protocol (RFC-0002 §7): a monotonic instant wrapper, a 4-timestamp
//! NTP-style epoch estimator, and a rolling statistics ring buffer.
//!
//! # Architecture
//!
//! The crate contains three modules:
//!
//! - [`instant`] — [`MonoInstant`]: a thin wrapper over [`std::time::Instant`]
//!   with arithmetic trait implementations (`Add<Duration>`, `Sub<Duration>`,
//!   `Sub<MonoInstant>`).
//! - [`epoch`] — [`ClockEpochEstimator`]: a 4-timestamp NTP/PTP monotonic clock
//!   offset estimator that converts viewer-local vsync timestamps to the host's
//!   local time domain (RFC-0002 §7.2).
//! - [`stats`] — [`RollingStats`]: a stack-allocated, fixed-capacity ring buffer
//!   for computing rolling mean, variance, and percentile estimates without heap
//!   allocation.
//!
//! # Usage
//!
//! ```rust
//! use renderd_clock::{MonoInstant, RollingStats};
//! use std::time::Duration;
//!
//! // Track encode latency samples for the EMA used in phase-sync scheduling
//! let mut stats = RollingStats::<64>::new();
//! stats.push(Duration::from_micros(7_200).as_micros() as u64);
//! stats.push(Duration::from_micros(8_100).as_micros() as u64);
//!
//! println!("mean encode latency: {:.0} µs", stats.mean());
//! ```
//!
//! # Design Note
//!
//! Like `renderd-abr`, all types in this crate are **pure state machines** driven
//! by the application layer. No threads, timers, or async runtimes are spawned here.
//!
//! # Panics
//!
//! [`MonoInstant::duration_since`] panics if the argument is later than `self` —
//! matching the contract of [`std::time::Instant::duration_since`]. Use
//! [`MonoInstant::checked_duration_since`] to avoid panics in fallible contexts.
//!
//! All other operations in this crate do not panic.

pub mod epoch;
pub mod instant;
pub mod stats;

pub use epoch::*;
pub use instant::*;
pub use stats::*;
