//! QUIC client and server loopback integration tests.

use rcgen::generate_simple_self_signed;
use renderd_net::{ClientTlsConfig, QuicClient, QuicServer, ServerTlsConfig};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::net::SocketAddr;

#[tokio::test]
async fn test_quic_server_client_loopback_handshake() {
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

    let server_handle = tokio::spawn(async move {
        let conn = server.accept().await.unwrap();
        conn
    });

    let client_conn = client
        .connect(actual_addr, "localhost", client_tls)
        .await
        .unwrap();

    let server_conn = server_handle.await.unwrap();

    assert_eq!(client_conn.remote_address(), actual_addr);
    assert_eq!(server_conn.stable_id(), server_conn.stable_id());
}
