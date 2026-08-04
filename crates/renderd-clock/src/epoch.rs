//! 4-timestamp NTP/PTP monotonic clock offset estimator per RFC-0002 §13.2.

/// Raw 4-timestamp sample collected during a clock synchronization ping-pong exchange.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockSample {
    /// Host send time in nanoseconds (host clock).
    pub t1_ns: u64,
    /// Viewer receive time in nanoseconds (viewer clock).
    pub t2_ns: u64,
    /// Viewer response send time in nanoseconds (viewer clock).
    pub t3_ns: u64,
    /// Host response receive time in nanoseconds (host clock).
    pub t4_ns: u64,
}

impl ClockSample {
    /// Computes the clock offset $\theta$ and round-trip time $\text{RTT}$ for this single sample.
    #[must_use]
    pub fn calculate(&self) -> ClockEstimate {
        let t1 = i128::from(self.t1_ns);
        let t2 = i128::from(self.t2_ns);
        let t3 = i128::from(self.t3_ns);
        let t4 = i128::from(self.t4_ns);

        // offset = ((t2 - t1) + (t3 - t4)) / 2
        let offset = ((t2 - t1) + (t3 - t4)) / 2;

        // rtt = (t4 - t1) - (t3 - t2)
        let rtt = (t4 - t1) - (t3 - t2);
        let rtt_ns = if rtt < 0 {
            0
        } else {
            u64::try_from(rtt).unwrap_or(u64::MAX)
        };

        ClockEstimate {
            offset_ns: i64::try_from(offset).unwrap_or(0),
            rtt_ns,
        }
    }
}

/// Estimated monotonic clock offset and round-trip time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockEstimate {
    /// Estimated clock offset in nanoseconds (`viewer_time - host_time`).
    pub offset_ns: i64,
    /// Estimated network round-trip time in nanoseconds.
    pub rtt_ns: u64,
}

/// Windowed minimum-RTT NTP/PTP clock epoch estimator.
#[derive(Debug, Clone)]
pub struct ClockEpochEstimator {
    samples: Vec<ClockSample>,
    capacity: usize,
}

impl ClockEpochEstimator {
    /// Creates a new `ClockEpochEstimator` maintaining up to `capacity` recent samples.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            samples: Vec::with_capacity(capacity),
            capacity,
        }
    }

    /// Adds a 4-timestamp exchange sample to the sliding window.
    pub fn add_sample(&mut self, sample: ClockSample) {
        if self.samples.len() >= self.capacity {
            self.samples.remove(0);
        }
        self.samples.push(sample);
    }

    /// Computes the optimal clock estimate by selecting the sample with the minimum RTT.
    #[must_use]
    pub fn estimate(&self) -> Option<ClockEstimate> {
        self.samples
            .iter()
            .map(ClockSample::calculate)
            .min_by_key(|e| e.rtt_ns)
    }

    /// Clears all historical samples.
    pub fn clear(&mut self) {
        self.samples.clear();
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn test_perfect_sync_zero_rtt() {
        let sample = ClockSample {
            t1_ns: 1000,
            t2_ns: 1000,
            t3_ns: 1000,
            t4_ns: 1000,
        };
        let estimate = sample.calculate();
        assert_eq!(estimate.offset_ns, 0);
        assert_eq!(estimate.rtt_ns, 0);
    }

    #[test]
    fn test_known_offset_and_rtt() {
        // Host sends t1=100. Viewer receives at t2=160 (+50ms viewer offset + 10ms transit delay).
        // Viewer replies t3=170 (+10ms processing). Host receives at t4=130 (+10ms transit delay).
        let sample = ClockSample {
            t1_ns: 100,
            t2_ns: 160,
            t3_ns: 170,
            t4_ns: 130,
        };
        let estimate = sample.calculate();
        assert_eq!(estimate.offset_ns, 50);
        assert_eq!(estimate.rtt_ns, 20);
    }
}
