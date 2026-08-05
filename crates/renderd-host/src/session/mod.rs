//! Host session state machine and device registry module scaffold.

pub mod auth;
pub mod devices;
pub mod pairing;

/// Host session state manager scaffold.
#[derive(Debug, Default)]
pub struct HostSession;

impl HostSession {
    /// Create a new host session scaffold.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}
