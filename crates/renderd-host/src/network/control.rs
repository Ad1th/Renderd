//! Control stream 0 message dispatch scaffold.

/// Host control stream dispatcher scaffold.
#[derive(Debug, Default)]
pub struct ControlDispatcher;

impl ControlDispatcher {
    /// Create a new control dispatcher scaffold.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}
