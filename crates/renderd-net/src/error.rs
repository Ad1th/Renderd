//! Error types for the `renderd-net` crate.

use thiserror::Error;

/// Error type for networking, TLS, and QUIC transport operations.
#[derive(Debug, Error)]
pub enum NetError {
    /// TLS configuration or handshake error.
    #[error("TLS configuration error: {0}")]
    Tls(String),

    /// QUIC connection error.
    #[error("QUIC connection error: {0}")]
    Connection(String),

    /// Framing serialization or deserialization error.
    #[error("Framing error: {0}")]
    Framing(String),

    /// Datagram send or receive error.
    #[error("Datagram I/O error: {0}")]
    Datagram(String),

    /// Underlying I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
