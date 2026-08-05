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
#[derive(Debug, Default)]
pub struct HostSession {
    state: SessionState,
}

impl HostSession {
    /// Creates a new `HostSession` in the [`SessionState::Idle`] state.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: SessionState::Idle,
        }
    }

    /// Returns a reference to the current session state.
    #[must_use]
    pub const fn state(&self) -> &SessionState {
        &self.state
    }

    /// Returns `true` if the session is in [`SessionState::Idle`].
    #[must_use]
    pub const fn is_idle(&self) -> bool {
        matches!(self.state, SessionState::Idle)
    }

    /// Returns `true` if the session is in [`SessionState::Streaming`].
    #[must_use]
    pub const fn is_streaming(&self) -> bool {
        matches!(self.state, SessionState::Streaming { .. })
    }

    /// Returns the connected viewer ID if the session is in `Connected` or `Streaming` state.
    #[must_use]
    pub const fn viewer_id(&self) -> Option<&ViewerId> {
        match &self.state {
            SessionState::Connected { viewer_id, .. }
            | SessionState::Streaming { viewer_id, .. } => Some(viewer_id),
            _ => None,
        }
    }

    /// Transitions from [`SessionState::Idle`] to [`SessionState::Pairing`].
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::InvalidTransition`] if the session is not currently `Idle`.
    pub fn begin_pairing(&mut self) -> Result<(), SessionError> {
        if self.state != SessionState::Idle {
            return Err(SessionError::InvalidTransition {
                from: self.state.to_string(),
                to: "PAIRING".to_string(),
            });
        }
        self.state = SessionState::Pairing;
        Ok(())
    }

    /// Transitions from [`SessionState::Pairing`] to [`SessionState::Connected`].
    ///
    /// Called once SPAKE2+ pairing succeeds and the viewer identity is established.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::InvalidTransition`] if the session is not currently `Pairing`.
    pub fn complete_pairing(
        &mut self,
        viewer_id: ViewerId,
        remote_addr: SocketAddr,
    ) -> Result<(), SessionError> {
        if self.state != SessionState::Pairing {
            return Err(SessionError::InvalidTransition {
                from: self.state.to_string(),
                to: "CONNECTED".to_string(),
            });
        }
        self.state = SessionState::Connected {
            viewer_id,
            remote_addr,
        };
        Ok(())
    }

    /// Transitions from [`SessionState::Connected`] to [`SessionState::Streaming`].
    ///
    /// Called when the viewer sends the first streaming request and the capture
    /// pipeline has been successfully started.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::InvalidTransition`] if the session is not currently `Connected`.
    pub fn begin_streaming(&mut self) -> Result<(), SessionError> {
        let (viewer_id, remote_addr) = match &self.state {
            SessionState::Connected {
                viewer_id,
                remote_addr,
            } => (*viewer_id, *remote_addr),
            _ => {
                return Err(SessionError::InvalidTransition {
                    from: self.state.to_string(),
                    to: "STREAMING".to_string(),
                });
            }
        };
        self.state = SessionState::Streaming {
            viewer_id,
            remote_addr,
        };
        Ok(())
    }

    /// Resets the session to [`SessionState::Idle`], regardless of current state.
    ///
    /// This is the universal error / disconnect handler. All state is cleared.
    pub fn reset(&mut self) {
        self.state = SessionState::Idle;
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
        assert_eq!(*session.state(), SessionState::Idle);
        assert!(session.is_idle());
    }

    #[test]
    fn test_full_lifecycle_idle_to_streaming() {
        let mut session = HostSession::new();
        let viewer_id = test_viewer_id();
        let addr = test_addr();

        // IDLE → PAIRING
        session.begin_pairing().expect("begin_pairing from Idle");
        assert_eq!(*session.state(), SessionState::Pairing);

        // PAIRING → CONNECTED
        session
            .complete_pairing(viewer_id, addr)
            .expect("complete_pairing from Pairing");
        assert!(matches!(session.state(), SessionState::Connected { .. }));
        assert_eq!(session.viewer_id(), Some(&viewer_id));

        // CONNECTED → STREAMING
        session
            .begin_streaming()
            .expect("begin_streaming from Connected");
        assert!(session.is_streaming());
        assert_eq!(session.viewer_id(), Some(&viewer_id));
    }

    #[test]
    fn test_reset_from_streaming_returns_to_idle() {
        let mut session = HostSession::new();
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
    fn test_invalid_transition_pairing_from_idle() {
        let mut session = HostSession::new();
        session.begin_pairing().unwrap();

        // Cannot pair again while already in Pairing state
        let err = session.begin_pairing().unwrap_err();
        assert!(matches!(err, SessionError::InvalidTransition { .. }));
    }

    #[test]
    fn test_invalid_transition_streaming_from_idle() {
        let mut session = HostSession::new();
        let err = session.begin_streaming().unwrap_err();
        assert!(matches!(err, SessionError::InvalidTransition { .. }));
    }

    #[test]
    fn test_invalid_transition_complete_pairing_from_idle() {
        let mut session = HostSession::new();
        let err = session
            .complete_pairing(test_viewer_id(), test_addr())
            .unwrap_err();
        assert!(matches!(err, SessionError::InvalidTransition { .. }));
    }

    #[test]
    fn test_state_display() {
        assert_eq!(SessionState::Idle.to_string(), "IDLE");
        assert_eq!(SessionState::Pairing.to_string(), "PAIRING");
    }
}
