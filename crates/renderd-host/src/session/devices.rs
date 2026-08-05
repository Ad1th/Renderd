//! Known-viewers device registry and revocation scaffold.

/// Paired devices registry scaffold.
#[derive(Debug, Default)]
pub struct DeviceRegistry;

impl DeviceRegistry {
    /// Create a new device registry scaffold.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}
