//! Application state and metrics management.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// Active network connection state of the viewer client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConnectionState {
    /// Disconnected from host daemon.
    #[default]
    Disconnected,
    /// Discovering or resolving host peer address via mDNS or manual IP.
    Discovering,
    /// Performing SPAKE2+ handshake and TLS 1.3 certificate setup.
    Handshaking,
    /// Connected and actively receiving video datagram stream.
    Connected,
    /// Connection lost; attempting peer reconnection.
    Reconnecting,
}

/// Thread-safe runtime metrics and application performance statistics counters.
#[derive(Debug, Default)]
pub struct ViewerMetrics {
    /// Total video frame packets received over the network.
    pub frames_received: AtomicU64,
    /// Total frames decoded by the video decoder engine.
    pub frames_decoded: AtomicU64,
    /// Total frames rendered and presented to the display swapchain.
    pub frames_rendered: AtomicU64,
    /// Total frames dropped due to queue congestion or stale presentation timestamps.
    pub frames_dropped: AtomicU64,
    /// Latest measured round-trip latency in microseconds.
    pub rtt_us: AtomicU64,
}

/// Shared central application state container.
#[derive(Debug, Clone)]
pub struct AppState {
    connection_state: Arc<std::sync::RwLock<ConnectionState>>,
    metrics: Arc<ViewerMetrics>,
    is_running: Arc<AtomicBool>,
    start_time: Instant,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    /// Creates a new [`AppState`] container initialized to default values.
    #[must_use]
    pub fn new() -> Self {
        Self {
            connection_state: Arc::new(std::sync::RwLock::new(ConnectionState::Disconnected)),
            metrics: Arc::new(ViewerMetrics::default()),
            is_running: Arc::new(AtomicBool::new(true)),
            start_time: Instant::now(),
        }
    }

    /// Gets the current [`ConnectionState`].
    #[must_use]
    pub fn connection_state(&self) -> ConnectionState {
        *self
            .connection_state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Sets the [`ConnectionState`].
    pub fn set_connection_state(&self, state: ConnectionState) {
        if let Ok(mut guard) = self.connection_state.write() {
            *guard = state;
        }
    }

    /// Returns a reference to shared [`ViewerMetrics`].
    #[must_use]
    pub fn metrics(&self) -> &ViewerMetrics {
        &self.metrics
    }

    /// Checks if the application is running.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.is_running.load(Ordering::Relaxed)
    }

    /// Signals the application to stop execution.
    pub fn stop(&self) {
        self.is_running.store(false, Ordering::SeqCst);
    }

    /// Returns the uptime duration since application launch.
    #[must_use]
    pub fn uptime(&self) -> std::time::Duration {
        self.start_time.elapsed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_state_lifecycle() {
        let state = AppState::new();
        assert_eq!(state.connection_state(), ConnectionState::Disconnected);
        state.set_connection_state(ConnectionState::Connected);
        assert_eq!(state.connection_state(), ConnectionState::Connected);
        assert!(state.is_running());
        state.stop();
        assert!(!state.is_running());
    }
}
