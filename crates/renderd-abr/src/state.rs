//! Adaptive Bitrate (ABR) congestion state machine per RFC-0002 §14.1.

/// Operational states of the Adaptive Bitrate congestion controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum AbrState {
    /// Operating normally at current target bitrate.
    #[default]
    Steady,
    /// Probing for higher bandwidth capacity.
    ProbeUp,
    /// Backing off target bitrate due to packet loss or delay jitter.
    Backoff,
    /// Severe packet loss or complete frame stall triggering keyframe request.
    Panic,
}

impl AbrState {
    /// Evaluates current network metrics and returns the next target state.
    #[must_use]
    #[allow(clippy::float_cmp)]
    pub const fn next_state(
        self,
        loss_rate: f64,
        loss_threshold: f64,
        panic_threshold: f64,
        consecutive_clean_intervals: usize,
        probe_trigger_intervals: usize,
    ) -> Self {
        if loss_rate >= panic_threshold {
            Self::Panic
        } else if loss_rate > loss_threshold {
            Self::Backoff
        } else if consecutive_clean_intervals >= probe_trigger_intervals {
            Self::ProbeUp
        } else {
            Self::Steady
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn test_state_transitions() {
        let state = AbrState::Steady;

        // Clean network -> ProbeUp
        assert_eq!(state.next_state(0.0, 0.02, 0.10, 5, 5), AbrState::ProbeUp);

        // Moderate loss -> Backoff
        assert_eq!(state.next_state(0.05, 0.02, 0.10, 0, 5), AbrState::Backoff);

        // Severe loss -> Panic
        assert_eq!(state.next_state(0.15, 0.02, 0.10, 0, 5), AbrState::Panic);
    }
}
