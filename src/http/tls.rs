//! TLS via rustls (ring backend) with bundled webpki roots. `insecure`
//! disables certificate verification for portals with self-signed certs.

use super::Stream;
use crate::error::{Error, Result};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, ClientConnection, DigitallySignedStruct, RootCertStore, StreamOwned};
use std::net::TcpStream;
use std::sync::Arc;

impl Stream for StreamOwned<ClientConnection, TcpStream> {}

#[derive(Debug)]
struct NoVerify(rustls::crypto::CryptoProvider);

impl ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

fn config(insecure: bool) -> Arc<ClientConfig> {
    let provider = rustls::crypto::ring::default_provider();
    let builder = ClientConfig::builder_with_provider(Arc::new(provider.clone()))
        .with_safe_default_protocol_versions()
        .expect("ring supports the default protocol versions");
    let cfg = if insecure {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoVerify(provider)))
            .with_no_client_auth()
    } else {
        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        builder.with_root_certificates(roots).with_no_client_auth()
    };
    Arc::new(cfg)
}

pub fn wrap(tcp: TcpStream, host: &str, insecure: bool) -> Result<Box<dyn Stream>> {
    let name = ServerName::try_from(host.to_string())
        .map_err(|_| Error::usage(format!("invalid tls server name: {host}")))?;
    let conn = ClientConnection::new(config(insecure), name)
        .map_err(|e| Error::network(format!("tls setup: {e}")))?;
    Ok(Box::new(StreamOwned::new(conn, tcp)))
}
