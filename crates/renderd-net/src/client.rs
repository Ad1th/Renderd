//! QUIC client wrapper for initiating Renderd peer connections.

use quinn::Endpoint;
use rustls::ClientConfig;
use std::net::SocketAddr;
use std::sync::Arc;

use crate::error::NetError;

/// Wrapper around a [`quinn::Endpoint`] operating as a client.
pub struct QuicClient {
    endpoint: Endpoint,
}

impl QuicClient {
    /// Binds an unbound client endpoint to a local ephemeral UDP port (`127.0.0.1:0` or `0.0.0.0:0`).
    ///
    /// # Errors
    /// Returns [`NetError`] if socket binding fails.
    pub fn bind_ephemeral() -> Result<Self, NetError> {
        let bind_addr = SocketAddr::from(([0, 0, 0, 0], 0));
        let endpoint = Endpoint::client(bind_addr)
            .map_err(|e| NetError::Connection(format!("Failed to bind client endpoint: {e}")))?;

        Ok(Self { endpoint })
    }

    /// Initiates a QUIC connection to a remote server address using the specified TLS configuration and server name.
    ///
    /// # Errors
    /// Returns [`NetError`] if connection initiation or TLS handshake fails.
    pub async fn connect(
        &self,
        addr: SocketAddr,
        server_name: &str,
        tls_config: ClientConfig,
    ) -> Result<quinn::Connection, NetError> {
        let crypto =
            quinn::crypto::rustls::QuicClientConfig::try_from(tls_config).map_err(|e| {
                NetError::Tls(format!("Failed to convert TLS client config for QUIC: {e}"))
            })?;
        let mut client_config = quinn::ClientConfig::new(Arc::new(crypto));

        let mut transport = quinn::TransportConfig::default();
        transport.max_concurrent_bidi_streams(100_u32.into());
        transport.max_concurrent_uni_streams(100_u32.into());
        transport.max_idle_timeout(Some(quinn::VarInt::from_u32(10_000).into()));
        transport.keep_alive_interval(Some(std::time::Duration::from_secs(2)));
        transport.datagram_receive_buffer_size(Some(2 * 1024 * 1024));
        client_config.transport_config(Arc::new(transport));

        let connecting = self
            .endpoint
            .connect_with(client_config, addr, server_name)
            .map_err(|e| {
                NetError::Connection(format!("Failed to initiate QUIC connection to {addr}: {e}"))
            })?;

        let connection = connecting.await.map_err(|e| {
            NetError::Connection(format!("QUIC client handshake failed with {addr}: {e}"))
        })?;

        Ok(connection)
    }

    /// Returns the local socket address this client endpoint is bound to.
    ///
    /// # Errors
    /// Returns [`NetError`] if retrieving the local socket address fails.
    pub fn local_addr(&self) -> Result<SocketAddr, NetError> {
        self.endpoint.local_addr().map_err(NetError::Io)
    }

    /// Closes the client endpoint with an error code and reason message.
    pub fn close(&self, error_code: u32, reason: &[u8]) {
        self.endpoint
            .close(quinn::VarInt::from_u32(error_code), reason);
    }
}
