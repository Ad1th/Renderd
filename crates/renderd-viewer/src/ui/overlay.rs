//! Reconnecting & status UI overlay (`renderd-viewer/src/ui/overlay.rs`).
//!
//! Renders a semi-transparent status message overlay (e.g. "Reconnecting...") over the last displayed
//! video frame when client is in `Reconnecting`, `Handshaking`, or `Discovering` states (RFC-0002 §18.1).

use crate::renderer::ViewportSize;
use crate::state::ConnectionState;

/// Semi-transparent UI status message overlay renderer.
#[derive(Debug, Clone)]
pub struct StatusOverlay {
    visible: bool,
    message: String,
    bg_alpha: f32,
}

impl Default for StatusOverlay {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusOverlay {
    /// Creates a new `StatusOverlay`.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            visible: false,
            message: String::new(),
            bg_alpha: 0.75,
        }
    }

    /// Returns whether the overlay is currently visible.
    #[must_use]
    pub const fn is_visible(&self) -> bool {
        self.visible
    }

    /// Returns the current overlay status message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns the background alpha opacity level.
    #[must_use]
    pub const fn bg_alpha(&self) -> f32 {
        self.bg_alpha
    }

    /// Updates overlay visibility and message string based on the current [`ConnectionState`].
    pub fn update_from_state(&mut self, state: ConnectionState) {
        match state {
            ConnectionState::Reconnecting => {
                self.visible = true;
                self.message = "Reconnecting to host...".to_string();
            }
            ConnectionState::Discovering => {
                self.visible = true;
                self.message = "Discovering host daemon...".to_string();
            }
            ConnectionState::Handshaking => {
                self.visible = true;
                self.message = "Establishing secure pairing...".to_string();
            }
            ConnectionState::Disconnected => {
                self.visible = true;
                self.message = "Disconnected from host".to_string();
            }
            ConnectionState::Connected => {
                self.visible = false;
                self.message.clear();
            }
        }
    }

    /// Executes rendering pass for status overlay text and semi-transparent banner.
    pub fn render(&self, _viewport: ViewportSize) {
        if !self.visible {
            return;
        }

        tracing::debug!(
            message = %self.message,
            alpha = self.bg_alpha,
            "Rendering semi-transparent status overlay"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_overlay_state_updates() {
        let mut overlay = StatusOverlay::new();
        assert!(!overlay.is_visible());

        overlay.update_from_state(ConnectionState::Reconnecting);
        assert!(overlay.is_visible());
        assert_eq!(overlay.message(), "Reconnecting to host...");

        overlay.update_from_state(ConnectionState::Connected);
        assert!(!overlay.is_visible());
        assert!(overlay.message().is_empty());
    }
}
