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

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn test_config_error_display() {
        let err_fnf = ConfigError::FileNotFound("/etc/renderd.toml".to_string());
        assert_eq!(
            format!("{err_fnf}"),
            "Configuration file not found: /etc/renderd.toml"
        );

        let err_parse = ConfigError::ParseError("invalid key".to_string());
        assert_eq!(
            format!("{err_parse}"),
            "Failed to parse configuration: invalid key"
        );

        let err_val = ConfigError::ValidationError {
            field: "host.target_fps",
            reason: "out of range".to_string(),
        };
        assert_eq!(
            format!("{err_val}"),
            "Invalid configuration field 'host.target_fps': out of range"
        );
    }
}
