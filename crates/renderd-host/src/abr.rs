//! Adaptive Bitrate (ABR) controller integration for `renderd-host`.
//!
//! Receives control-plane telemetry (`ReactiveStats`, `PeriodicStats`, `KeyframeRequest`)
//! from the connected viewer and drives the `AbrEngine` to adjust encoder bitrate and
//! request IDR keyframes.

use std::sync::{Arc, Mutex};

use renderd_abr::{AbrEngine, BitrateDecision};
use renderd_proto::generated::renderd::{PeriodicStats, ReactiveStats};
use renderd_proto::types::BitrateKbps;

use crate::encode::EncodePipeline;
use crate::error::HostError;

/// Manager for host-side ABR decision processing.
#[derive(Debug, Clone)]
pub struct AbrManager {
    engine: Arc<Mutex<AbrEngine>>,
}

impl Default for AbrManager {
    fn default() -> Self {
        Self::new()
    }
}

impl AbrManager {
    /// Creates a new `AbrManager` with default 1080p60 parameters:
    /// min = 15,000 Kbps (15 Mbps), max = 60,000 Kbps (60 Mbps), initial = 35,000 Kbps (35 Mbps).
    #[must_use]
    pub fn new() -> Self {
        Self::with_bounds(
            BitrateKbps(15_000),
            BitrateKbps(60_000),
            BitrateKbps(35_000),
            BitrateKbps(3_000),
        )
    }

    /// Creates an `AbrManager` with explicit parameter bounds.
    #[must_use]
    pub fn with_bounds(
        min_bitrate: BitrateKbps,
        max_bitrate: BitrateKbps,
        initial_bitrate: BitrateKbps,
        step: BitrateKbps,
    ) -> Self {
        let engine = AbrEngine::new(
            min_bitrate,
            max_bitrate,
            initial_bitrate,
            step,
            0.05, // 5% loss triggers backoff
            0.20, // 20% loss triggers panic
        );
        Self {
            engine: Arc::new(Mutex::new(engine)),
        }
    }

    /// Processes a short-term [`ReactiveStats`] report (100 ms loop) from the viewer.
    ///
    /// Updates `AbrEngine` with the loss rate. If the decision calls for a bitrate change
    /// or keyframe, updates `encode_pipeline`.
    ///
    /// # Errors
    /// Returns [`HostError::Initialization`] if `encode_pipeline.set_bitrate` fails.
    pub fn on_reactive_stats(
        &self,
        stats: &ReactiveStats,
        pipeline: &EncodePipeline,
    ) -> Result<BitrateDecision, HostError> {
        let loss_rate = f64::from(stats.loss_rate.clamp(0.0, 1.0));

        let mut engine = self
            .engine
            .lock()
            .map_err(|_| HostError::Initialization("AbrManager mutex poisoned".into()))?;

        let decision = engine.update(loss_rate);
        drop(engine);

        // Apply decision to encoder pipeline
        pipeline.set_bitrate(decision.target_bitrate_kbps.0)?;

        if decision.request_keyframe {
            pipeline.force_keyframe();
        }

        tracing::debug!(
            loss_rate = stats.loss_rate,
            target_kbps = decision.target_bitrate_kbps.0,
            request_keyframe = decision.request_keyframe,
            "Applied ABR reactive stats decision"
        );

        Ok(decision)
    }

    /// Processes a long-term [`PeriodicStats`] report (500 ms loop) from the viewer.
    ///
    /// # Errors
    /// Returns [`HostError::Initialization`] if updating bitrate on `encode_pipeline` fails.
    pub fn on_periodic_stats(
        &self,
        stats: &PeriodicStats,
        pipeline: &EncodePipeline,
    ) -> Result<BitrateDecision, HostError> {
        let rx_bw_kbps = stats.receive_bandwidth_kbps;

        // If receive bandwidth is reported (> 0), evaluate if current bitrate exceeds 80% of estimated bandwidth
        if rx_bw_kbps > 0.0 {
            let mut engine = self
                .engine
                .lock()
                .map_err(|_| HostError::Initialization("AbrManager mutex poisoned".into()))?;

            let current = engine.current_bitrate().0;
            // SAFETY/LINT: rx_bw_kbps is checked > 0.0 above, making sign loss impossible.
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            let target_from_bw = (rx_bw_kbps * 0.8) as u32;

            if target_from_bw < current && target_from_bw >= 1_000 {
                // High loss / bandwidth degradation detected
                let decision = engine.update(0.10); // simulate 10% degradation
                drop(engine);
                pipeline.set_bitrate(decision.target_bitrate_kbps.0)?;
                return Ok(decision);
            }
        }

        let mut engine = self
            .engine
            .lock()
            .map_err(|_| HostError::Initialization("AbrManager mutex poisoned".into()))?;
        let decision = engine.update(0.0);
        drop(engine);

        Ok(decision)
    }

    /// Triggers an immediate explicit keyframe request on `encode_pipeline`.
    pub fn on_keyframe_request(&self, pipeline: &EncodePipeline) {
        pipeline.force_keyframe();
    }

    /// Returns the currently active target bitrate in Kbps.
    ///
    /// # Panics
    /// Panics if the internal `Mutex` is poisoned.
    #[must_use]
    pub fn current_bitrate(&self) -> BitrateKbps {
        self.engine
            .lock()
            .expect("AbrManager mutex poisoned")
            .current_bitrate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode::EncodePipeline;

    #[test]
    fn test_abr_manager_reactive_stats_loss_reduces_bitrate() {
        let manager = AbrManager::new();
        let pipeline = EncodePipeline::new();

        let initial_bw = manager.current_bitrate();
        assert_eq!(initial_bw.0, 35_000);

        // Send ReactiveStats with 10% loss rate (above 5% loss threshold)
        let stats = ReactiveStats {
            loss_rate: 0.10,
            jitter_us: 100,
            last_frame_id: 1,
        };

        let decision = manager.on_reactive_stats(&stats, &pipeline).unwrap();
        assert!(
            decision.target_bitrate_kbps.0 < initial_bw.0,
            "Bitrate should be reduced on 10% loss: got {:?}",
            decision.target_bitrate_kbps
        );
    }

    #[test]
    fn test_abr_manager_keyframe_request() {
        let manager = AbrManager::new();
        let pipeline = EncodePipeline::new();
        let rx = pipeline.receiver();

        manager.on_keyframe_request(&pipeline);
        pipeline
            .push_frame(bytes::Bytes::from_static(b"frame"), 0)
            .unwrap();

        let frame = rx.try_recv().unwrap();
        assert!(frame.is_keyframe);
    }
}
