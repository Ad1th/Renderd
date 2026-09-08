//! QUIC client wrapper for initiating Renderd peer connections.

use quinn::{Endpoint, EndpointConfig};
use rustls::ClientConfig;
use std::net::SocketAddr;
use std::sync::Arc;

use crate::error::NetError;
use crate::transport::{bind_udp, transport_config};

/// Wrapper around a [`quinn::Endpoint`] operating as a client.
pub struct QuicClient {
    endpoint: Endpoint,
}

impl QuicClient {
    /// Binds an unbound client endpoint to a local ephemeral UDP port (`0.0.0.0:0`).
    ///
    /// # Errors
    /// Returns [`NetError`] if socket binding fails.
    pub fn bind_ephemeral() -> Result<Self, NetError> {
        let bind_addr = SocketAddr::from(([0, 0, 0, 0], 0));
        let socket = bind_udp(bind_addr)
            .map_err(|e| NetError::Connection(format!("Failed to bind client socket: {e}")))?;
        let runtime = quinn::default_runtime()
            .ok_or_else(|| NetError::Connection("no async runtime available".to_string()))?;
        let endpoint = Endpoint::new(EndpointConfig::default(), None, socket, runtime)
            .map_err(|e| NetError::Connection(format!("Failed to bind client endpoint: {e}")))?;

        Ok(Self { endpoint })
    }

    /// Initiates a QUIC connection to a remote server address using the specified TLS
    /// configuration and server name, starting at the 1200-byte minimum MTU.
    ///
    /// # Errors
    /// Returns [`NetError`] if connection initiation or TLS handshake fails.
    pub async fn connect(
        &self,
        addr: SocketAddr,
        server_name: &str,
        tls_config: ClientConfig,
    ) -> Result<quinn::Connection, NetError> {
        self.connect_with_mtu(addr, server_name, tls_config, 1200)
            .await
    }

    /// Initiates a QUIC connection, starting path MTU at `initial_mtu` bytes.
    ///
    /// # Errors
    /// Returns [`NetError`] if connection initiation or TLS handshake fails.
    pub async fn connect_with_mtu(
        &self,
        addr: SocketAddr,
        server_name: &str,
        tls_config: ClientConfig,
        initial_mtu: u16,
    ) -> Result<quinn::Connection, NetError> {
        let crypto =
            quinn::crypto::rustls::QuicClientConfig::try_from(tls_config).map_err(|e| {
                NetError::Tls(format!("Failed to convert TLS client config for QUIC: {e}"))
            })?;
        let mut client_config = quinn::ClientConfig::new(Arc::new(crypto));
        client_config.transport_config(Arc::new(transport_config(initial_mtu)));

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
