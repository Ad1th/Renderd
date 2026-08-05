//! macOS menu bar UI scaffold.

/// Status bar menu item scaffold.
#[derive(Debug, Default)]
pub struct MenuBar;

impl MenuBar {
    /// Create a new menu bar scaffold.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}
