//! Platform credential storage interface for Renderd.

pub mod entry;
pub mod error;
pub mod store;

pub use entry::PairingEntry;
pub use error::KeychainError;
pub use store::KeychainStore;
