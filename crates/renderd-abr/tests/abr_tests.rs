//! Integration and property-based tests for ABR engine state machine.

use proptest::prelude::*;
use renderd_proto::types::BitrateKbps;

use renderd_abr::{AbrEngine, AbrState, Signals};

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

    /// Whatever sequence of loss, delay, receive rate and idleness arrives, the
    /// bitrate stays in bounds and a keyframe is only ever requested because of
    /// loss, never because of delay alone.
    #[test]
    fn test_abr_signals_proptest(
        reports in prop::collection::vec(
            (0.0f64..=1.0, 0.0f64..1_000.0, 0.0f64..500.0, 0.0f64..20_000.0, any::<bool>()),
            1..200,
        )
    ) {
        let min_b = BitrateKbps(1_500);
        let max_b = BitrateKbps(10_000);
        let mut engine = AbrEngine::new(min_b, max_b, BitrateKbps(4_000), BitrateKbps(1_000), 0.02, 0.15);
        for (loss, queue, send_queue, rate, idle) in reports {
            let decision = engine.update_signals(&Signals {
                loss_rate: loss,
                queue_delay_ms: queue,
                send_queue_ms: send_queue,
                receive_rate_kbps: rate,
                app_limited: idle,
            });
            prop_assert!(decision.target_bitrate_kbps >= min_b);
            prop_assert!(decision.target_bitrate_kbps <= max_b);
            if decision.request_keyframe {
                prop_assert!(loss >= 0.15);
            }
        }
    }
}
