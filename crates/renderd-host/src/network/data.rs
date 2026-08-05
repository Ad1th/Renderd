//! Datagram burst sender task scaffold.

/// Host datagram burst sender scaffold.
#[derive(Debug, Default)]
pub struct DataSender;

impl DataSender {
    /// Create a new data sender scaffold.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}
