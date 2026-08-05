//! Host SPAKE2+ pairing handler scaffold.

/// Host pairing handler scaffold.
#[derive(Debug, Default)]
pub struct PairingHandler;

impl PairingHandler {
    /// Create a new pairing handler scaffold.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}
