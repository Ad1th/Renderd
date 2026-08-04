//! `ConfigError` hierarchy definitions for `renderd-config`.

use thiserror::Error;

/// Error type returned during configuration loading or validation failures.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    /// Configuration file not found at path.
    #[error("Configuration file not found: {0}")]
    FileNotFound(String),

    /// Deserialization or parsing syntax error.
    #[error("Failed to parse configuration: {0}")]
    ParseError(String),

    /// Semantic validation rule violation.
    #[error("Invalid configuration field '{field}': {reason}")]
    ValidationError {
        /// Configuration field name.
        field: &'static str,
        /// Detail of validation rule failure.
        reason: String,
    },
}
