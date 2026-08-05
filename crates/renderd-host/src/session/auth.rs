//! Host authentication and certificate validation scaffold.

/// Host authentication manager scaffold.
#[derive(Debug, Default)]
pub struct AuthManager;

impl AuthManager {
    /// Create a new authentication manager scaffold.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}
