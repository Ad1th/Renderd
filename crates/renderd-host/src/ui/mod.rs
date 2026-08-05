//! User interface module scaffold for macOS host agent.

pub mod devices_panel;
pub mod menubar;
pub mod notifications;

/// Host UI manager scaffold.
#[derive(Debug, Default)]
pub struct UiManager;

impl UiManager {
    /// Create a new UI manager scaffold.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}
