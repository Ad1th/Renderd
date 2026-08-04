//! TLS configuration builders for Renderd server and client transport endpoints.

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{ClientConfig, ServerConfig, SignatureScheme};
use std::sync::Arc;

use crate::error::NetError;

/// Custom client certificate verifier that validates against a pinned certificate DER.
#[derive(Debug)]
struct PinnedClientCertVerifier {
    pinned_cert: Vec<u8>,
    supported_algs: rustls::crypto::WebPkiSupportedAlgorithms,
}

impl ClientCertVerifier for PinnedClientCertVerifier {
    fn offer_client_auth(&self) -> bool {
        true
    }

    fn client_auth_mandatory(&self) -> bool {
        true
    }

    fn root_hint_subjects(&self) -> &[rustls::DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        if end_entity.as_ref() == self.pinned_cert.as_slice() {
            Ok(ClientCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::UnknownIssuer,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::PeerIncompatible(
            rustls::PeerIncompatible::Tls12NotOffered,
        ))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.supported_algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.supported_algs.supported_schemes()
    }
}

/// Custom server certificate verifier that validates against a pinned certificate DER.
#[derive(Debug)]
struct PinnedServerCertVerifier {
    pinned_cert: Vec<u8>,
    supported_algs: rustls::crypto::WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for PinnedServerCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if end_entity.as_ref() == self.pinned_cert.as_slice() {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::UnknownIssuer,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::PeerIncompatible(
            rustls::PeerIncompatible::Tls12NotOffered,
        ))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.supported_algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.supported_algs.supported_schemes()
    }
}

/// Builder for QUIC server TLS configuration.
pub struct ServerTlsConfig;

impl ServerTlsConfig {
    /// Creates a [`ServerConfig`] configured for TLS 1.3 only, using the given server certificate and private key.
    ///
    /// If `client_cert` is provided, mutual TLS (mTLS) is enforced using pinned certificate verification.
    ///
    /// # Errors
    /// Returns [`NetError::Tls`] if TLS configuration fails.
    pub fn from_cert(
        cert_chain: Vec<CertificateDer<'static>>,
        key: PrivateKeyDer<'static>,
        client_cert: Option<CertificateDer<'static>>,
    ) -> Result<ServerConfig, NetError> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());

        let builder = ServerConfig::builder_with_provider(provider.clone())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|e| NetError::Tls(format!("Failed to set TLS 1.3 protocol versions: {e}")))?;

        let builder = if let Some(pinned) = client_cert {
            let verifier = Arc::new(PinnedClientCertVerifier {
                pinned_cert: pinned.as_ref().to_vec(),
                supported_algs: provider.signature_verification_algorithms,
            });
            builder.with_client_cert_verifier(verifier)
        } else {
            builder.with_no_client_auth()
        };

        let mut config = builder
            .with_single_cert(cert_chain, key)
            .map_err(|e| NetError::Tls(format!("Failed to set server certificate: {e}")))?;

        config.alpn_protocols = vec![b"renderd-v1".to_vec()];
        Ok(config)
    }
}

/// Builder for QUIC client TLS configuration.
pub struct ClientTlsConfig;

impl ClientTlsConfig {
    /// Creates a [`ClientConfig`] configured for TLS 1.3 only, pinned to the given server certificate.
    ///
    /// If `client_cert` and private key are provided, client mutual TLS authentication is included.
    ///
    /// # Errors
    /// Returns [`NetError::Tls`] if TLS configuration fails.
    #[allow(clippy::needless_pass_by_value)]
    pub fn with_pinned_cert(
        client_cert: Option<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)>,
        server_cert: CertificateDer<'static>,
    ) -> Result<ClientConfig, NetError> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());

        let verifier = Arc::new(PinnedServerCertVerifier {
            pinned_cert: server_cert.to_vec(),
            supported_algs: provider.signature_verification_algorithms,
        });

        let builder = ClientConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|e| NetError::Tls(format!("Failed to set TLS 1.3 protocol versions: {e}")))?
            .dangerous()
            .with_custom_certificate_verifier(verifier);

        let mut config = if let Some((certs, key)) = client_cert {
            builder
                .with_client_auth_cert(certs, key)
                .map_err(|e| NetError::Tls(format!("Failed to set client certificate: {e}")))?
        } else {
            builder.with_no_client_auth()
        };

        config.alpn_protocols = vec![b"renderd-v1".to_vec()];
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::generate_simple_self_signed;

    #[test]
    fn test_tls_config_initialization() {
        let cert_gen = generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let cert_der = CertificateDer::from(cert_gen.cert.der().to_vec());
        let key_der = PrivateKeyDer::Pkcs8(cert_gen.key_pair.serialize_der().into());

        let server_config = ServerTlsConfig::from_cert(
            vec![cert_der.clone()],
            key_der.clone_key(),
            Some(cert_der.clone()),
        )
        .unwrap();
        assert_eq!(server_config.alpn_protocols, vec![b"renderd-v1".to_vec()]);

        let client_config =
            ClientTlsConfig::with_pinned_cert(Some((vec![cert_der.clone()], key_der)), cert_der)
                .unwrap();
        assert_eq!(client_config.alpn_protocols, vec![b"renderd-v1".to_vec()]);
    }
}
