//! Error types for protocol envelope validation and message handling.

use thiserror::Error;

/// Errors arising during protocol message validation or serialization.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProtoError {
    /// Required field is missing or empty.
    #[error("Missing required field: {0}")]
    MissingField(&'static str),

    /// Invalid field value or out-of-range parameter.
    #[error("Invalid field '{field}': {reason}")]
    InvalidValue {
        /// Name of the field containing invalid data.
        field: &'static str,
        /// Explanation of why the value is invalid.
        reason: String,
    },

    /// Protocol version incompatibility.
    #[error("Incompatible protocol version: peer requires {required}, supported is {supported}")]
    IncompatibleVersion {
        /// Minimum protocol version required by peer.
        required: u32,
        /// Local supported protocol version.
        supported: u32,
    },
}
