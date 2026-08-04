//! Error types for the `renderd-keychain` crate.

use thiserror::Error;

/// Error type for credential and keychain storage operations.
#[derive(Debug, Error)]
pub enum KeychainError {
    /// Pairing entry was not found in store.
    #[error("Pairing entry not found for peer {0}")]
    NotFound(String),

    /// Store I/O or platform keychain failure.
    #[error("Keychain platform failure: {0}")]
    Platform(String),

    /// Serialization or deserialization error.
    #[error("Serialization error: {0}")]
    Serialization(String),
}
