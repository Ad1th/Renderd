//! Host application state machine and run loop.
//!
//! `HostApp` initializes and orchestrates all host subsystems:
//! - Screen capture pipeline (`CapturePipeline`)
//! - Hardware video encoder (`EncodePipeline`)
//! - Presentation clock controller (`ClockController`)
//! - Adaptive bitrate controller (`AbrManager`)
//! - Session lifecycle manager (`HostSession`)
//! - Network manager (`NetworkManager`)
//! - UI manager (`UiManager`)
//!
//! The run loop blocks until the process receives SIGINT or SIGTERM.

use std::sync::Arc;

use crate::abr::AbrManager;
use crate::capture::CapturePipeline;
use crate::clock::ClockController;
use crate::encode::EncodePipeline;
use crate::error::HostError;
use crate::network::NetworkManager;
use crate::session::{HostSession, SessionState};
use crate::ui::UiManager;

/// Main host application orchestrator.
///
/// Owns all long-lived subsystem components and drives the run loop until shutdown.
#[derive(Debug)]
pub struct HostApp {
    capture: CapturePipeline,
    encode: Arc<EncodePipeline>,
    clock: ClockController,
    abr: AbrManager,
    session: HostSession,
    network: NetworkManager,
    ui: UiManager,
}

impl Default for HostApp {
    fn default() -> Self {
        Self::new()
    }
}

impl HostApp {
    /// Creates a new `HostApp` with all subsystems in their default initialized states.
    #[must_use]
    pub fn new() -> Self {
        Self {
            capture: CapturePipeline::new(),
            encode: Arc::new(EncodePipeline::new()),
            clock: ClockController::new(),
            abr: AbrManager::new(),
            session: HostSession::new(),
            network: NetworkManager::new(),
            ui: UiManager::new(),
        }
    }

    /// Runs the host application run loop.
    ///
    /// Initializes all subsystems, transitions the session to `IDLE` (listening state),
    /// and blocks until the process receives SIGINT or SIGTERM.
    ///
    /// # Errors
    ///
    /// Returns a [`HostError`] if any subsystem fails to initialize or the session
    /// is in an unexpected state on entry.
    pub fn run(&mut self) -> Result<(), HostError> {
        tracing::info!("renderd-host starting subsystem initialization");

        // Verify session starts in Idle — fail fast if state is corrupted.
        if !matches!(self.session.state(), SessionState::Idle) {
            return Err(HostError::Initialization(format!(
                "HostSession expected Idle on startup, got {}",
                self.session.state()
            )));
        }

        // Log capture pipeline status (real start is deferred until a viewer connects)
        tracing::info!(
            capture_running = self.capture.is_running(),
            "Capture pipeline ready (starts on first viewer connection)"
        );

        // Confirm encode pipeline is ready (receiver handle proves the ring buffer is live)
        let _encode_rx = self.encode.receiver();
        tracing::info!("Encode pipeline ready");

        // Log initial clock controller target interval (60 Hz default)
        let clock_interval = self.clock.target_interval();
        tracing::info!(
            target_interval_ns = clock_interval.as_nanos(),
            "Presentation clock controller initialized"
        );

        // Log initial ABR target bitrate
        let initial_bitrate = self.abr.current_bitrate();
        tracing::info!(
            initial_bitrate_kbps = initial_bitrate.0,
            "ABR controller initialized"
        );

        // Confirm network manager is ready
        tracing::info!(
            network_manager = ?self.network,
            "Network manager initialized"
        );

        // Confirm UI manager is ready and post startup notification
        self.ui.notifications.notify_session_started("renderd-host");
        tracing::info!("UI manager initialized (menu bar and notifications)");

        tracing::info!(
            session_state = %self.session.state(),
            "Session state machine ready — host is listening for viewer connections"
        );

        tracing::info!(
            "renderd-host subsystems initialized — entering run loop (press Ctrl+C to stop)"
        );

        // Block the current thread until SIGINT or SIGTERM is received.
        // All real work (QUIC server, capture, encoding) will run on background threads/tasks
        // spawned by the respective subsystems when fully wired in future milestones.
        Self::wait_for_shutdown()
    }

    /// Blocks until SIGINT (Ctrl+C) or SIGTERM is received.
    fn wait_for_shutdown() -> Result<(), HostError> {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let stop = Arc::new(AtomicBool::new(false));

        // Register handlers for SIGINT and SIGTERM using ctrlc crate semantics via std hooks.
        let stop_clone = Arc::clone(&stop);
        ctrlc::set_handler(move || {
            tracing::info!("Received shutdown signal — stopping renderd-host");
            stop_clone.store(true, Ordering::SeqCst);
        })
        .map_err(|e| HostError::Initialization(format!("Failed to install signal handler: {e}")))?;

        // Park main thread until signal fires
        while !stop.load(Ordering::SeqCst) {
            std::thread::park_timeout(std::time::Duration::from_millis(250));
        }

        tracing::info!("renderd-host shutdown complete");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_host_app_instantiation() {
        let app = HostApp::new();
        assert!(matches!(app.session.state(), SessionState::Idle));
    }
}
