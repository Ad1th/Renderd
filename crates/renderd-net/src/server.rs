//! QUIC server wrapper for listening for incoming Renderd peer connections.

use quinn::Endpoint;
use rustls::ServerConfig;
use std::net::SocketAddr;
use std::sync::Arc;

use crate::error::NetError;

/// Wrapper around a [`quinn::Endpoint`] operating as a server.
pub struct QuicServer {
    endpoint: Endpoint,
}

impl QuicServer {
    /// Binds a QUIC server endpoint to the specified address with the given TLS configuration.
    ///
    /// # Errors
    /// Returns [`NetError`] if the socket binding or TLS configuration fails.
    pub fn bind(addr: SocketAddr, tls_config: ServerConfig) -> Result<Self, NetError> {
        let crypto =
            quinn::crypto::rustls::QuicServerConfig::try_from(tls_config).map_err(|e| {
                NetError::Tls(format!("Failed to convert TLS server config for QUIC: {e}"))
            })?;
        let mut server_config = quinn::ServerConfig::with_crypto(Arc::new(crypto));

        let mut transport = quinn::TransportConfig::default();
        transport.max_concurrent_bidi_streams(100_u32.into());
        transport.max_concurrent_uni_streams(100_u32.into());
        transport.max_idle_timeout(Some(quinn::VarInt::from_u32(10_000).into()));
        transport.keep_alive_interval(Some(std::time::Duration::from_secs(2)));
        transport.datagram_receive_buffer_size(Some(2 * 1024 * 1024));
        server_config.transport_config(Arc::new(transport));

        let endpoint = Endpoint::server(server_config, addr).map_err(|e| {
            NetError::Connection(format!(
                "Failed to bind QUIC server endpoint on {addr}: {e}"
            ))
        })?;

        Ok(Self { endpoint })
    }

    /// Returns the local socket address this server endpoint is bound to.
    ///
    /// # Errors
    /// Returns [`NetError`] if retrieving the local socket address fails.
    pub fn local_addr(&self) -> Result<SocketAddr, NetError> {
        self.endpoint.local_addr().map_err(NetError::Io)
    }

    /// Accepts an incoming connection from a peer.
    ///
    /// # Errors
    /// Returns [`NetError`] if connection handshake fails or server endpoint is closed.
    pub async fn accept(&self) -> Result<quinn::Connection, NetError> {
        let incoming = self
            .endpoint
            .accept()
            .await
            .ok_or_else(|| NetError::Connection("QUIC server endpoint closed".to_string()))?;

        let connection = incoming
            .await
            .map_err(|e| NetError::Connection(format!("QUIC connection handshake failed: {e}")))?;

        Ok(connection)
    }

    /// Closes the server endpoint with an error code and reason message.
    pub fn close(&self, error_code: u32, reason: &[u8]) {
        self.endpoint
            .close(quinn::VarInt::from_u32(error_code), reason);
    }
}
