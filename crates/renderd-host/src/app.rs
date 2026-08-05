//! Host application state machine scaffold.

use crate::error::HostError;

/// Main host application orchestrator.
#[derive(Debug, Default)]
pub struct HostApp;

impl HostApp {
    /// Create a new host application instance.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Run the host application event loop scaffold.
    ///
    /// # Errors
    ///
    /// Returns a [`HostError`] if initialization or runtime execution fails.
    #[allow(clippy::unused_self)]
    pub const fn run(&self) -> Result<(), HostError> {
        Ok(())
    }
}
