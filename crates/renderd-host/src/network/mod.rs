//! Network transport and control stream module scaffold.

pub mod control;
pub mod data;
pub mod server;

/// Host network manager scaffold.
#[derive(Debug, Default)]
pub struct NetworkManager;

impl NetworkManager {
    /// Create a new network manager scaffold.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}
