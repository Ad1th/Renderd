//! Adaptive Bitrate (ABR) controller integration scaffold.

/// Host ABR controller manager scaffold.
#[derive(Debug, Default)]
pub struct AbrManager;

impl AbrManager {
    /// Create a new ABR manager scaffold.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}
