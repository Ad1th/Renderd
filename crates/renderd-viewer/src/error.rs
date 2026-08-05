//! Error types for the `renderd-viewer` crate.

use thiserror::Error;

/// Central error type for the Renderd Viewer application.
#[derive(Debug, Error)]
pub enum ViewerError {
    /// Window creation or event loop error.
    #[error("Window error: {0}")]
    Window(String),

    /// Graphics renderer error.
    #[error("Renderer error: {0}")]
    Renderer(String),

    /// Video decoder error.
    #[error("Decoder error: {0}")]
    Decoder(String),

    /// Configuration error.
    #[error("Configuration error: {0}")]
    Config(String),

    /// Network or protocol error.
    #[error("Network error: {0}")]
    Network(String),

    /// I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_viewer_error_display() {
        let err = ViewerError::Window("creation failed".to_string());
        assert_eq!(err.to_string(), "Window error: creation failed");
    }
}
