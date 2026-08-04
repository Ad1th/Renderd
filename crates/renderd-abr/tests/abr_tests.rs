//! Integration and property-based tests for ABR engine state machine.

use proptest::prelude::*;
use renderd_proto::types::BitrateKbps;

use renderd_abr::{AbrEngine, AbrState};

#[test]
fn test_panic_state_keyframe_request() {
    let mut engine = AbrEngine::new(
        BitrateKbps(5000),
        BitrateKbps(50000),
        BitrateKbps(20000),
        BitrateKbps(2000),
        0.02,
        0.10,
    );

    let decision = engine.update(0.15); // Severe loss 15%
    assert_eq!(decision.state, AbrState::Panic);
    assert!(decision.request_keyframe);
    assert_eq!(decision.target_bitrate_kbps, BitrateKbps(10000));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn test_abr_bitrate_bounds_proptest(
        loss_sequence in prop::collection::vec(0.0f64..=1.0f64, 1..100)
    ) {
        let min_b = BitrateKbps(5000);
        let max_b = BitrateKbps(50000);
        let mut engine = AbrEngine::new(
            min_b,
            max_b,
            BitrateKbps(25000),
            BitrateKbps(2000),
            0.02,
            0.10,
        );

        for loss in loss_sequence {
            let decision = engine.update(loss);
            prop_assert!(decision.target_bitrate_kbps >= min_b);
            prop_assert!(decision.target_bitrate_kbps <= max_b);
        }
    }
}
