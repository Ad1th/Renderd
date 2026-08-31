//! Fixed-capacity stack-allocated rolling statistics ring buffer.

/// Fixed-capacity stack-allocated rolling statistics ring buffer backing zero-allocation statistics.
#[derive(Debug, Clone)]
pub struct RollingStats<const N: usize> {
    buffer: [u64; N],
    head: usize,
    count: usize,
}

impl<const N: usize> Default for RollingStats<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> RollingStats<N> {
    /// Creates a new empty `RollingStats` ring buffer.
    ///
    /// A window of `N == 0` is rejected at compile time; it would divide by zero and
    /// index an empty array on the first `push`.
    #[must_use]
    pub const fn new() -> Self {
        const { assert!(N > 0, "RollingStats window size must be greater than 0") };
        Self {
            buffer: [0; N],
            head: 0,
            count: 0,
        }
    }

    /// Records a new sample into the rolling window, overwriting the oldest sample when full.
    pub fn push(&mut self, sample: u64) {
        self.buffer[self.head] = sample;
        self.head = (self.head + 1) % N;
        if self.count < N {
            self.count += 1;
        }
    }

    /// Returns the number of active samples stored in the buffer (`0 ..= N`).
    #[must_use]
    pub const fn len(&self) -> usize {
        self.count
    }

    /// Returns `true` if the buffer contains 0 samples.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Returns the arithmetic mean of recorded samples, or `0.0` if empty.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn mean(&self) -> f64 {
        if self.count == 0 {
            return 0.0;
        }
        let sum: u64 = self.buffer[..self.count].iter().sum();
        sum as f64 / self.count as f64
    }

    /// Returns the population variance (dividing by `n`, not `n - 1`), or `0.0` if
    /// fewer than two samples have been recorded.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn variance(&self) -> f64 {
        if self.count < 2 {
            return 0.0;
        }
        let m = self.mean();
        let sum_sq_diff: f64 = self.buffer[..self.count]
            .iter()
            .map(|&val| {
                let diff = val as f64 - m;
                diff * diff
            })
            .sum();
        sum_sq_diff / self.count as f64
    }

    /// Returns the population standard deviation, the square root of [`Self::variance`].
    #[must_use]
    pub fn std_dev(&self) -> f64 {
        self.variance().sqrt()
    }

    /// Returns the minimum sample value, or `None` if empty.
    #[must_use]
    pub fn min(&self) -> Option<u64> {
        if self.count == 0 {
            None
        } else {
            self.buffer[..self.count].iter().copied().min()
        }
    }

    /// Returns the maximum sample value, or `None` if empty.
    #[must_use]
    pub fn max(&self) -> Option<u64> {
        if self.count == 0 {
            None
        } else {
            self.buffer[..self.count].iter().copied().max()
        }
    }

    /// Computes the specified percentile `p` (`0.0 ..= 100.0`), or `0` if empty.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn percentile(&self, p: f64) -> u64 {
        if self.count == 0 {
            return 0;
        }
        let mut sorted = [0u64; N];
        sorted[..self.count].copy_from_slice(&self.buffer[..self.count]);
        sorted[..self.count].sort_unstable();

        let p_clamped = p.clamp(0.0, 100.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let rank = ((p_clamped / 100.0) * (self.count - 1) as f64).round() as usize;
        sorted[rank]
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn test_rolling_stats_basic() {
        let mut stats = RollingStats::<5>::new();
        assert!(stats.is_empty());
        assert_eq!(stats.len(), 0);

        stats.push(10);
        stats.push(20);
        stats.push(30);

        assert_eq!(stats.len(), 3);
        assert!((stats.mean() - 20.0).abs() < 1e-6);
        assert_eq!(stats.min(), Some(10));
        assert_eq!(stats.max(), Some(30));
        assert_eq!(stats.percentile(50.0), 20);
    }
}
