//! Network transport and control stream module scaffold.

pub mod control;
pub mod data;
pub mod pressure;
pub mod retransmit;
pub mod server;

pub use control::ControlDispatcher;
pub use data::DataSender;
pub use pressure::LinkPressure;
pub use retransmit::RetransmitCache;

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
