//! Host session state machine managing lifecycle transitions for connected viewer sessions.
//!
//! The state machine implements the following transitions:
//!
//! ```text
//! IDLE ──► PAIRING ──► CONNECTED ──► STREAMING
//!   ▲          │            │              │
//!   └──────────┘            │              │
//!   ▲                       └──────────────┘
//!   └─────── (disconnect / error resets to IDLE) ──────────┘
//! ```

pub mod auth;
pub mod devices;
pub mod pairing;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use renderd_proto::types::ViewerId;

/// Current operational state of the host session.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum SessionState {
    /// No viewer is connected; host is listening for incoming connections.
    #[default]
    Idle,
    /// A viewer has connected and SPAKE2+ pairing is in progress.
    Pairing,
    /// Pairing completed; viewer is authenticated and the control channel is established.
    Connected {
        /// The authenticated viewer's UUID.
        viewer_id: ViewerId,
        /// Network address of the connected viewer.
        remote_addr: SocketAddr,
    },
    /// Active screen share is running; frames are being encoded and transmitted.
    Streaming {
        /// The authenticated viewer's UUID.
        viewer_id: ViewerId,
        /// Network address of the connected viewer.
        remote_addr: SocketAddr,
    },
}

impl std::fmt::Display for SessionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Idle => write!(f, "IDLE"),
            Self::Pairing => write!(f, "PAIRING"),
            Self::Connected { viewer_id, .. } => write!(f, "CONNECTED({})", viewer_id.0),
            Self::Streaming { viewer_id, .. } => write!(f, "STREAMING({})", viewer_id.0),
        }
    }
}

/// Errors that can occur during session state transitions.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SessionError {
    /// A state transition was requested from an incompatible current state.
    #[error("invalid transition from {from} to {to}")]
    InvalidTransition {
        /// The state we are transitioning from.
        from: String,
        /// The state we attempted to transition to.
        to: String,
    },
}

/// Host session lifecycle manager.
///
/// Tracks the current session state and connected viewer details, enforcing
/// valid state transitions for the `IDLE → PAIRING → CONNECTED → STREAMING`
/// lifecycle defined in RFC-0002 §9.
///
/// Holds inner state behind an `Arc<Mutex<SessionState>>` so it can be safely cloned
/// and updated across tokio tasks, QUIC event loops, and UI components.
#[derive(Debug, Clone)]
pub struct HostSession {
    state: Arc<Mutex<SessionState>>,
}

impl Default for HostSession {
    fn default() -> Self {
        Self::new()
    }
}

impl HostSession {
    /// Creates a new `HostSession` in the [`SessionState::Idle`] state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(SessionState::Idle)),
        }
    }

    /// Returns a snapshot of the current session state.
    ///
    /// # Panics
    /// Panics if the internal state mutex is poisoned.
    #[must_use]
    pub fn state(&self) -> SessionState {
        self.state
            .lock()
            .expect("HostSession mutex poisoned")
            .clone()
    }

    /// Returns `true` if the session is in [`SessionState::Idle`].
    ///
    /// # Panics
    /// Panics if the internal state mutex is poisoned.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        matches!(
            *self.state.lock().expect("HostSession mutex poisoned"),
            SessionState::Idle
        )
    }

    /// Returns `true` if the session is in [`SessionState::Streaming`].
    ///
    /// # Panics
    /// Panics if the internal state mutex is poisoned.
    #[must_use]
    pub fn is_streaming(&self) -> bool {
        matches!(
            *self.state.lock().expect("HostSession mutex poisoned"),
            SessionState::Streaming { .. }
        )
    }

    /// Returns the connected viewer ID if the session is in `Connected` or `Streaming` state.
    ///
    /// # Panics
    /// Panics if the internal state mutex is poisoned.
    #[must_use]
    pub fn viewer_id(&self) -> Option<ViewerId> {
        match *self.state.lock().expect("HostSession mutex poisoned") {
            SessionState::Connected { viewer_id, .. }
            | SessionState::Streaming { viewer_id, .. } => Some(viewer_id),
            _ => None,
        }
    }

    /// Transitions from [`SessionState::Idle`] to [`SessionState::Pairing`].
    ///
    /// # Errors
    /// Returns [`SessionError::InvalidTransition`] if the session is not currently `Idle`.
    ///
    /// # Panics
    /// Panics if the internal state mutex is poisoned.
    pub fn begin_pairing(&self) -> Result<(), SessionError> {
        let mut guard = self.state.lock().expect("HostSession mutex poisoned");
        if *guard != SessionState::Idle {
            return Err(SessionError::InvalidTransition {
                from: guard.to_string(),
                to: "PAIRING".to_string(),
            });
        }
        let from = guard.to_string();
        *guard = SessionState::Pairing;
        drop(guard);

        tracing::info!(from = %from, to = "PAIRING", "HostSession state transition: IDLE -> PAIRING");
        Ok(())
    }

    /// Transitions from [`SessionState::Idle`] or [`SessionState::Pairing`] to [`SessionState::Connected`].
    ///
    /// Called once Stream 0 negotiation or SPAKE2+ pairing succeeds and viewer identity is established.
    ///
    /// # Errors
    /// Returns [`SessionError::InvalidTransition`] if session is not in `Idle` or `Pairing`.
    ///
    /// # Panics
    /// Panics if the internal state mutex is poisoned.
    pub fn complete_pairing(
        &self,
        viewer_id: ViewerId,
        remote_addr: SocketAddr,
    ) -> Result<(), SessionError> {
        let mut guard = self.state.lock().expect("HostSession mutex poisoned");
        if *guard != SessionState::Idle && *guard != SessionState::Pairing {
            return Err(SessionError::InvalidTransition {
                from: guard.to_string(),
                to: "CONNECTED".to_string(),
            });
        }
        let from = guard.to_string();
        *guard = SessionState::Connected {
            viewer_id,
            remote_addr,
        };
        let to = guard.to_string();
        drop(guard);

        tracing::info!(
            from = %from,
            to = %to,
            %viewer_id,
            %remote_addr,
            "HostSession state transition to CONNECTED"
        );
        Ok(())
    }

    /// Transitions from [`SessionState::Connected`] to [`SessionState::Streaming`].
    ///
    /// Called when the viewer requests streaming and the capture pipeline is started.
    ///
    /// # Errors
    /// Returns [`SessionError::InvalidTransition`] if session is not in `Connected`.
    ///
    /// # Panics
    /// Panics if the internal state mutex is poisoned.
    pub fn begin_streaming(&self) -> Result<(), SessionError> {
        let mut guard = self.state.lock().expect("HostSession mutex poisoned");
        let SessionState::Connected {
            viewer_id,
            remote_addr,
        } = *guard
        else {
            return Err(SessionError::InvalidTransition {
                from: guard.to_string(),
                to: "STREAMING".to_string(),
            });
        };
        let from = guard.to_string();
        *guard = SessionState::Streaming {
            viewer_id,
            remote_addr,
        };
        let to = guard.to_string();
        drop(guard);

        tracing::info!(
            from = %from,
            to = %to,
            %viewer_id,
            %remote_addr,
            "HostSession state transition to STREAMING"
        );
        Ok(())
    }

    /// Resets the session to [`SessionState::Idle`], regardless of current state.
    ///
    /// # Panics
    /// Panics if the internal state mutex is poisoned.
    pub fn reset(&self) {
        let mut guard = self.state.lock().expect("HostSession mutex poisoned");
        let from = guard.to_string();
        *guard = SessionState::Idle;
        drop(guard);

        tracing::info!(from = %from, to = "IDLE", "HostSession state transition to IDLE");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn test_addr() -> SocketAddr {
        "127.0.0.1:9000".parse().unwrap()
    }

    fn test_viewer_id() -> ViewerId {
        ViewerId(Uuid::new_v4())
    }

    #[test]
    fn test_initial_state_is_idle() {
        let session = HostSession::new();
        assert_eq!(session.state(), SessionState::Idle);
        assert!(session.is_idle());
    }

    #[test]
    fn test_full_lifecycle_idle_to_streaming() {
        let session = HostSession::new();
        let viewer_id = test_viewer_id();
        let addr = test_addr();

        // IDLE → PAIRING
        session.begin_pairing().expect("begin_pairing from Idle");
        assert_eq!(session.state(), SessionState::Pairing);

        // PAIRING → CONNECTED
        session
            .complete_pairing(viewer_id, addr)
            .expect("complete_pairing from Pairing");
        assert!(matches!(session.state(), SessionState::Connected { .. }));
        assert_eq!(session.viewer_id(), Some(viewer_id));

        // CONNECTED → STREAMING
        session
            .begin_streaming()
            .expect("begin_streaming from Connected");
        assert!(session.is_streaming());
        assert_eq!(session.viewer_id(), Some(viewer_id));
    }

    #[test]
    fn test_direct_idle_to_connected_reconnect() {
        let session = HostSession::new();
        let viewer_id = test_viewer_id();
        let addr = test_addr();

        // IDLE → CONNECTED (Direct connection for paired viewer)
        session.complete_pairing(viewer_id, addr).unwrap();
        assert!(matches!(session.state(), SessionState::Connected { .. }));
    }

    #[test]
    fn test_reset_from_streaming_returns_to_idle() {
        let session = HostSession::new();
        let viewer_id = test_viewer_id();
        let addr = test_addr();

        session.begin_pairing().unwrap();
        session.complete_pairing(viewer_id, addr).unwrap();
        session.begin_streaming().unwrap();

        session.reset();
        assert!(session.is_idle());
        assert_eq!(session.viewer_id(), None);
    }

    #[test]
    fn test_invalid_transition_pairing_from_pairing() {
        let session = HostSession::new();
        session.begin_pairing().unwrap();

        // Cannot pair again while already in Pairing state
        let err = session.begin_pairing().unwrap_err();
        assert!(matches!(err, SessionError::InvalidTransition { .. }));
    }

    #[test]
    fn test_invalid_transition_streaming_from_idle() {
        let session = HostSession::new();
        let err = session.begin_streaming().unwrap_err();
        assert!(matches!(err, SessionError::InvalidTransition { .. }));
    }

    #[test]
    fn test_state_display() {
        assert_eq!(SessionState::Idle.to_string(), "IDLE");
        assert_eq!(SessionState::Pairing.to_string(), "PAIRING");
    }
}
