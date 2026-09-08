//! QUIC server wrapper for listening for incoming Renderd peer connections.

use quinn::{Endpoint, EndpointConfig};
use rustls::ServerConfig;
use std::net::SocketAddr;
use std::sync::Arc;

use crate::error::NetError;
use crate::transport::{bind_udp, transport_config};

/// Wrapper around a [`quinn::Endpoint`] operating as a server.
pub struct QuicServer {
    endpoint: Endpoint,
}

impl QuicServer {
    /// Binds a QUIC server endpoint to the specified address with the given TLS configuration.
    ///
    /// Uses the 1200-byte QUIC minimum as the initial MTU; prefer
    /// [`QuicServer::bind_with_mtu`] when the configured MTU is known.
    ///
    /// # Errors
    /// Returns [`NetError`] if the socket binding or TLS configuration fails.
    pub fn bind(addr: SocketAddr, tls_config: ServerConfig) -> Result<Self, NetError> {
        Self::bind_with_mtu(addr, tls_config, 1200)
    }

    /// Binds a QUIC server endpoint, starting path MTU at `initial_mtu` bytes.
    ///
    /// # Errors
    /// Returns [`NetError`] if the socket binding or TLS configuration fails.
    pub fn bind_with_mtu(
        addr: SocketAddr,
        tls_config: ServerConfig,
        initial_mtu: u16,
    ) -> Result<Self, NetError> {
        let crypto =
            quinn::crypto::rustls::QuicServerConfig::try_from(tls_config).map_err(|e| {
                NetError::Tls(format!("Failed to convert TLS server config for QUIC: {e}"))
            })?;
        let mut server_config = quinn::ServerConfig::with_crypto(Arc::new(crypto));
        server_config.transport_config(Arc::new(transport_config(initial_mtu)));

        let socket = bind_udp(addr).map_err(|e| {
            NetError::Connection(format!("Failed to bind UDP socket on {addr}: {e}"))
        })?;
        let runtime = quinn::default_runtime()
            .ok_or_else(|| NetError::Connection("no async runtime available".to_string()))?;

        let endpoint = Endpoint::new(
            EndpointConfig::default(),
            Some(server_config),
            socket,
            runtime,
        )
        .map_err(|e| {
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
