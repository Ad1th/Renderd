//! QUIC server loop scaffold.

/// Host QUIC server scaffold.
#[derive(Debug, Default)]
pub struct HostServer;

impl HostServer {
    /// Create a new host server scaffold.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}
