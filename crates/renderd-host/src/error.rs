//! Error types for the `renderd-host` application.

use thiserror::Error;

/// Errors that can occur within the `renderd-host` application.
#[derive(Debug, Error)]
pub enum HostError {
    /// Configuration loading or validation failed.
    #[error("configuration error: {0}")]
    Config(#[from] renderd_config::ConfigError),

    /// Application initialization failed.
    #[error("initialization failed: {0}")]
    Initialization(String),
}
