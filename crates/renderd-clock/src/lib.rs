//! High-resolution monotonic clock wrapper and NTP/PTP offset estimator.

pub mod epoch;
pub mod instant;
pub mod stats;

pub use epoch::*;
pub use instant::*;
pub use stats::*;
