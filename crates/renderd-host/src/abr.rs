//! Adaptive Bitrate (ABR) controller integration for `renderd-host`.
//!
//! Receives control-plane telemetry (`ReactiveStats`, `PeriodicStats`, `KeyframeRequest`)
//! from the connected viewer and drives the `AbrEngine` to adjust encoder bitrate and
//! request IDR keyframes.
//!
//! Only measured loss drives the bitrate. An earlier version also capped the bitrate
//! at 80% of the viewer's *received* bandwidth, but on a still desktop the encoder
//! sends almost nothing, so "received bandwidth" collapsed and dragged the bitrate to
//! its floor — and the moment a video started playing it looked like a slideshow.

use std::sync::{Arc, Mutex};

use renderd_abr::{AbrEngine, BitrateDecision};
use renderd_config::{AbrConfig, HostConfig};
use renderd_proto::generated::renderd::{PeriodicStats, ReactiveStats};
use renderd_proto::types::BitrateKbps;

use crate::encode::EncodePipeline;
use crate::error::HostError;

/// Loss rate at which the engine halves the bitrate and forces a keyframe.
const PANIC_LOSS_RATE: f64 = 0.15;

/// Manager for host-side ABR decision processing.
#[derive(Debug, Clone)]
pub struct AbrManager {
    engine: Arc<Mutex<AbrEngine>>,
    last_keyframe_request: Arc<Mutex<std::time::Instant>>,
}

impl Default for AbrManager {
    fn default() -> Self {
        Self::new()
    }
}

impl AbrManager {
    /// Creates a new `AbrManager` with default 1080p60 parameters:
    /// min = 8,000 Kbps (8 Mbps), max = 25,000 Kbps (25 Mbps), initial = 15,000 Kbps (15 Mbps).
    #[must_use]
    pub fn new() -> Self {
        Self::with_bounds(
            BitrateKbps(8_000),
            BitrateKbps(25_000),
            BitrateKbps(15_000),
            BitrateKbps(2_000),
        )
    }

    /// Creates an `AbrManager` from the loaded configuration.
    ///
    /// The host's `max_bitrate_kbps` is the *starting* bitrate; the ABR range comes
    /// from the `[abr]` section, with the start clamped inside it.
    #[must_use]
    pub fn from_config(abr: &AbrConfig, host: &HostConfig) -> Self {
        let min = abr.min_bitrate_kbps.max(1_000);
        let max = abr.max_bitrate_kbps.max(min);
        let initial = host.max_bitrate_kbps.clamp(min, max);
        let step = abr.step_kbps.max(250);
        let engine = AbrEngine::new(
            BitrateKbps(min),
            BitrateKbps(max),
            BitrateKbps(initial),
            BitrateKbps(step),
            f64::from(abr.loss_threshold.clamp(0.001, 0.5)),
            PANIC_LOSS_RATE,
        );
        Self::from_engine(engine)
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
        Self::from_engine(engine)
    }

    fn from_engine(engine: AbrEngine) -> Self {
        let past = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(5))
            .unwrap_or_else(std::time::Instant::now);
        Self {
            engine: Arc::new(Mutex::new(engine)),
            last_keyframe_request: Arc::new(Mutex::new(past)),
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

        // `set_bitrate` is a no-op for an unchanged value, so this is cheap to call
        // on every tick.
        pipeline.set_bitrate(decision.target_bitrate_kbps.0)?;

        if decision.request_keyframe {
            self.on_keyframe_request(pipeline);
        }

        tracing::debug!(
            loss_rate = stats.loss_rate,
            target_kbps = decision.target_bitrate_kbps.0,
            state = ?decision.state,
            request_keyframe = decision.request_keyframe,
            "Applied ABR reactive stats decision"
        );

        Ok(decision)
    }

    /// Processes a long-term [`PeriodicStats`] report (500 ms loop) from the viewer.
    ///
    /// The periodic report is telemetry only; it is logged and does not steer the
    /// bitrate (see the module documentation for why).
    ///
    /// # Errors
    /// Returns [`HostError::Initialization`] if the engine mutex is poisoned.
    pub fn on_periodic_stats(
        &self,
        stats: &PeriodicStats,
        _pipeline: &EncodePipeline,
    ) -> Result<BitrateDecision, HostError> {
        let engine = self
            .engine
            .lock()
            .map_err(|_| HostError::Initialization("AbrManager mutex poisoned".into()))?;
        let decision = BitrateDecision {
            state: engine.state(),
            target_bitrate_kbps: engine.current_bitrate(),
            request_keyframe: false,
        };
        drop(engine);

        tracing::info!(
            viewer_rx_kbps = format!("{:.0}", stats.receive_bandwidth_kbps),
            decode_us = stats.decode_time_us,
            render_us = stats.render_time_us,
            frames_displayed = stats.frames_displayed,
            frames_dropped = stats.frames_dropped,
            target_kbps = decision.target_bitrate_kbps.0,
            "VIEWER TELEMETRY"
        );

        Ok(decision)
    }

    /// Triggers an explicit IDR keyframe on `encode_pipeline`, debounced to at most once per 250 ms.
    ///
    /// A keyframe already costs the link a burst; issuing another before the first
    /// one has even been decoded only makes the loss that triggered it worse.
    pub fn on_keyframe_request(&self, pipeline: &EncodePipeline) {
        if let Ok(mut last) = self.last_keyframe_request.lock() {
            if last.elapsed() >= std::time::Duration::from_millis(250) {
                *last = std::time::Instant::now();
                pipeline.force_keyframe();
                tracing::info!("AbrManager: dispatched IDR keyframe request to encoder");
            }
        }
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
        assert_eq!(initial_bw.0, 15_000);

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

    /// A quiet desktop reporting tiny received bandwidth must not drag the bitrate down.
    #[test]
    fn test_periodic_stats_do_not_steer_bitrate() {
        let manager = AbrManager::new();
        let pipeline = EncodePipeline::new();
        let before = manager.current_bitrate();
        let stats = PeriodicStats {
            receive_bandwidth_kbps: 300.0,
            decode_time_us: 1_000,
            render_time_us: 500,
            frames_displayed: 10,
            frames_dropped: 0,
        };
        manager.on_periodic_stats(&stats, &pipeline).unwrap();
        assert_eq!(manager.current_bitrate(), before);
    }

    #[test]
    fn test_from_config_clamps_initial_into_range() {
        let abr = AbrConfig {
            min_bitrate_kbps: 10_000,
            max_bitrate_kbps: 20_000,
            ..Default::default()
        };
        let host = HostConfig {
            max_bitrate_kbps: 50_000,
            ..Default::default()
        };
        let manager = AbrManager::from_config(&abr, &host);
        assert_eq!(manager.current_bitrate().0, 20_000);
    }
}
