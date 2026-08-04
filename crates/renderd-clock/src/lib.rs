//! High-resolution monotonic clock wrapper and NTP/PTP offset estimator.

pub mod epoch;
pub mod instant;

pub use epoch::*;
pub use instant::*;
