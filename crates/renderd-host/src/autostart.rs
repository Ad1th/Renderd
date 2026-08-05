//! macOS login item auto-start manager scaffold.

/// Auto-start service manager scaffold.
#[derive(Debug, Default)]
pub struct AutoStartManager;

impl AutoStartManager {
    /// Create a new auto-start manager scaffold.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}
