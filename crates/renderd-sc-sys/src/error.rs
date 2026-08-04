//! Typed error definitions for `ScreenCaptureKit` capture operations.
//!
//! This module defines [`ScError`], covering TCC permission failures, display enumeration
//! errors, content filter creation errors, and stream lifecycle failures.

use thiserror::Error;

/// Error type returned by `ScreenCaptureKit` operations.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ScError {
    /// Screen recording permission (TCC) was denied or has been restricted by system policy.
    #[error("Screen recording permission denied by macOS system settings (TCC)")]
    PermissionDenied,

    /// No active display was found on the system.
    #[error("No active displays found for screen capture")]
    NoDisplaysFound,

    /// Failed to build an `SCContentFilter` for the target display or application.
    #[error("Failed to create ScreenCaptureKit content filter: {0}")]
    FilterCreationFailed(String),

    /// Failed to initialize, start, or configure the `SCStream`.
    #[error("ScreenCaptureKit stream error: {0}")]
    StreamFailed(String),

    /// Provided stream configuration parameters are invalid.
    #[error("Invalid ScreenCaptureKit stream configuration: {0}")]
    InvalidConfiguration(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display_formatting() {
        assert_eq!(
            ScError::PermissionDenied.to_string(),
            "Screen recording permission denied by macOS system settings (TCC)"
        );

        assert_eq!(
            ScError::NoDisplaysFound.to_string(),
            "No active displays found for screen capture"
        );

        assert_eq!(
            ScError::FilterCreationFailed("display ID 1 not found".into()).to_string(),
            "Failed to create ScreenCaptureKit content filter: display ID 1 not found"
        );

        assert_eq!(
            ScError::StreamFailed("stream start timeout".into()).to_string(),
            "ScreenCaptureKit stream error: stream start timeout"
        );

        assert_eq!(
            ScError::InvalidConfiguration("fps must be > 0".into()).to_string(),
            "Invalid ScreenCaptureKit stream configuration: fps must be > 0"
        );
    }

    #[test]
    fn test_error_trait_impl() {
        let err: Box<dyn std::error::Error> = Box::new(ScError::PermissionDenied);
        assert!(err.to_string().contains("TCC"));
    }
}
