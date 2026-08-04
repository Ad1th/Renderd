//! Monotonic high-resolution instant wrapper per RFC-0002 §13.1.

use std::ops::{Add, AddAssign, Sub, SubAssign};
use std::time::{Duration, Instant};

/// High-resolution monotonic clock instant wrapper.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MonoInstant(pub Instant);

impl MonoInstant {
    /// Returns the current monotonic instant (`MonoInstant::now()`).
    #[must_use]
    pub fn now() -> Self {
        Self(Instant::now())
    }

    /// Returns the duration elapsed since this instant was captured.
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        self.0.elapsed()
    }

    /// Returns the duration elapsed between `earlier` and `self`.
    ///
    /// # Panics
    /// Panics if `earlier` is later than `self`.
    #[must_use]
    pub fn duration_since(&self, earlier: Self) -> Duration {
        self.0.duration_since(earlier.0)
    }

    /// Returns `Some(duration)` if `self >= earlier`, or `None` otherwise.
    #[must_use]
    pub fn checked_duration_since(&self, earlier: Self) -> Option<Duration> {
        self.0.checked_duration_since(earlier.0)
    }
}

impl Add<Duration> for MonoInstant {
    type Output = Self;

    fn add(self, rhs: Duration) -> Self::Output {
        Self(self.0 + rhs)
    }
}

impl AddAssign<Duration> for MonoInstant {
    fn add_assign(&mut self, rhs: Duration) {
        self.0 += rhs;
    }
}

impl Sub<Duration> for MonoInstant {
    type Output = Self;

    #[allow(clippy::unchecked_time_subtraction)]
    fn sub(self, rhs: Duration) -> Self::Output {
        Self(self.0 - rhs)
    }
}

impl SubAssign<Duration> for MonoInstant {
    fn sub_assign(&mut self, rhs: Duration) {
        self.0 -= rhs;
    }
}

impl Sub<Self> for MonoInstant {
    type Output = Duration;

    fn sub(self, rhs: Self) -> Self::Output {
        self.0.duration_since(rhs.0)
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use std::thread::sleep;

    #[test]
    fn test_mono_instant_monotonicity() {
        let t0 = MonoInstant::now();
        sleep(Duration::from_millis(5));
        let t1 = MonoInstant::now();

        assert!(t1 > t0);
        let elapsed = t1 - t0;
        assert!(elapsed >= Duration::from_millis(4));
    }
}
