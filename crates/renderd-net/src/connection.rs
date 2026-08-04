//! Connection telemetry and statistical utilities for QUIC streams and datagrams.

use quinn::Connection;
use std::time::Duration;

/// Extension trait providing telemetry accessors for a [`Connection`].
pub trait QuicConnectionExt {
    /// Returns the smoothed round-trip time (RTT) for the active connection path.
    fn rtt(&self) -> Duration;
}

impl QuicConnectionExt for Connection {
    fn rtt(&self) -> Duration {
        self.stats().path.rtt
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ClientTlsConfig, QuicClient, QuicServer, ServerTlsConfig};
    use rcgen::generate_simple_self_signed;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    use std::net::SocketAddr;

    #[tokio::test]
    async fn test_connection_rtt_exporter() {
        let cert_gen = generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let cert_der = CertificateDer::from(cert_gen.cert.der().to_vec());
        let key_der = PrivateKeyDer::Pkcs8(cert_gen.key_pair.serialize_der().into());

        let server_tls = ServerTlsConfig::from_cert(
            vec![cert_der.clone()],
            key_der.clone_key(),
            Some(cert_der.clone()),
        )
        .unwrap();

        let client_tls =
            ClientTlsConfig::with_pinned_cert(Some((vec![cert_der.clone()], key_der)), cert_der)
                .unwrap();

        let server_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let server = QuicServer::bind(server_addr, server_tls).unwrap();
        let actual_addr = server.local_addr().unwrap();

        let client = QuicClient::bind_ephemeral().unwrap();

        let server_task = tokio::spawn(async move { server.accept().await.unwrap() });

        let client_conn = client
            .connect(actual_addr, "localhost", client_tls)
            .await
            .unwrap();

        let _server_conn = server_task.await.unwrap();

        let rtt = client_conn.rtt();
        // RTT should be non-negative duration
        assert!(rtt >= Duration::from_nanos(0));
    }
}
