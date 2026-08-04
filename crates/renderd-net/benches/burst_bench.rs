//! Criterion benchmark for datagram burst sender.

#![allow(missing_docs)]

use bytes::Bytes;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rcgen::generate_simple_self_signed;
use renderd_net::{ClientTlsConfig, FragmentBurst, QuicClient, QuicServer, ServerTlsConfig};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::net::SocketAddr;
use tokio::runtime::Runtime;

fn bench_fragment_burst(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let (client_conn, _server_conn) = rt.block_on(async {
        let cert_gen = generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let cert_der = CertificateDer::from(cert_gen.cert.der().to_vec());
        let key_der = PrivateKeyDer::Pkcs8(cert_gen.key_pair.serialize_der().into());

        let server_tls = ServerTlsConfig::from_cert(
            vec![cert_der.clone()],
            key_der.clone_key(),
            Some(cert_der.clone()),
        )
        .unwrap();

        let client_tls = ClientTlsConfig::with_pinned_cert(
            Some((vec![cert_der.clone()], key_der)),
            cert_der,
        )
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

        let server_conn = server_task.await.unwrap();

        (client_conn, server_conn)
    });

    let fragments: Vec<Bytes> = (0u8..55u8)
        .map(|i| Bytes::from(vec![i; 1150]))
        .collect();

    c.bench_function("burst_send_55_fragments", |b| {
        b.iter(|| {
            let res = FragmentBurst::send_all(black_box(&client_conn), black_box(&fragments));
            black_box(res).ok();
        });
    });
}

criterion_group!(benches, bench_fragment_burst);
criterion_main!(benches);
