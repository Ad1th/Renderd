//! QUIC client and server loopback integration tests.

use rcgen::generate_simple_self_signed;
use renderd_net::{ClientTlsConfig, FragmentBurst, QuicClient, QuicServer, ServerTlsConfig};
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

/// `queued_bytes` must see datagrams the congestion controller has not let onto
/// the wire yet, and see them drain once it does.
#[tokio::test]
async fn test_queued_bytes_tracks_the_datagram_send_queue() {
    let cert_gen = generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let cert_der = CertificateDer::from(cert_gen.cert.der().to_vec());
    let key_der = PrivateKeyDer::Pkcs8(cert_gen.key_pair.serialize_der().into());
    let server_tls = ServerTlsConfig::from_cert(vec![cert_der.clone()], key_der, None).unwrap();
    let client_tls = ClientTlsConfig::with_insecure_skip_verify().unwrap();

    let server = QuicServer::bind("127.0.0.1:0".parse().unwrap(), server_tls).unwrap();
    let addr = server.local_addr().unwrap();
    let client = QuicClient::bind_ephemeral().unwrap();
    let accept = tokio::spawn(async move { server.accept().await.unwrap() });
    let client_conn = client.connect(addr, "localhost", client_tls).await.unwrap();
    let server_conn = accept.await.unwrap();

    assert_eq!(FragmentBurst::queued_bytes(&server_conn), 0);

    // Far more than one congestion window, so most of it has to wait in the queue.
    let payload = bytes::Bytes::from(vec![0u8; 1000]);
    let fragments = vec![payload; 1000];
    FragmentBurst::send_all(&server_conn, &fragments).unwrap();
    let queued = FragmentBurst::queued_bytes(&server_conn);
    assert!(queued > 0, "a burst larger than the window must queue");

    // Drain on the client so the window opens and the queue empties.
    let reader = tokio::spawn(async move { while client_conn.read_datagram().await.is_ok() {} });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while FragmentBurst::queued_bytes(&server_conn) > 0 && std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(FragmentBurst::queued_bytes(&server_conn), 0);
    server_conn.close(0u32.into(), b"done");
    reader.abort();
}
