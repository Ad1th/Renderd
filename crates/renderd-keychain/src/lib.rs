//! Platform credential storage interface for Renderd.

pub mod entry;
pub mod error;
#[cfg(target_os = "macos")]
pub mod macos;
pub mod mock;
pub mod store;
#[cfg(target_os = "windows")]
pub mod windows;

pub use entry::PairingEntry;
pub use error::KeychainError;
#[cfg(target_os = "macos")]
pub use macos::MacosKeychain;
pub use mock::MockKeychain;
pub use store::KeychainStore;
#[cfg(target_os = "windows")]
pub use windows::WindowsCredentialManager;
