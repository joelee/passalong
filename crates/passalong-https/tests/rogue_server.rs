//! A server that presents the pinned certificate without holding its
//! private key must be refused: the pin names a key, and only the key's
//! owner can sign the handshake. The certificate is public; anyone can send
//! it.

use std::path::Path;
use std::sync::Arc;

use passalong_core::api_key::ApiKey;
use passalong_core::config;
use passalong_core::testing::MapEnv;
use passalong_https::{Client, HttpsError};
use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::sign::{CertifiedKey, SingleCertAndKey};

/// The pinned server's certificate, and its pin.
const CERT: &str = include_str!("fixtures/self-signed.pem");
const CERT_PIN: &str = "sha256/yFchGu73Az8s4FnAfymiO33A6WswijiRgzkEfdjrHYE=";

/// A TLS server that sends `CERT` but signs with a key of its own.
fn impostor() -> rustls::ServerConfig {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &SystemRandom::new())
        .unwrap();
    let key = provider
        .key_provider
        .load_private_key(PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
            pkcs8.as_ref().to_vec(),
        )))
        .unwrap();
    let cert = CertificateDer::from_pem_slice(CERT.as_bytes()).unwrap();
    rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(SingleCertAndKey::from(CertifiedKey::new(
            vec![cert],
            key,
        ))))
}

#[tokio::test]
async fn a_server_with_the_pinned_certificate_but_not_its_key_is_refused() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(impostor()));
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            // The client gives up during the handshake.
            let _ = acceptor.accept(stream).await;
        }
    });

    let text = format!(
        "[client]\ndevice_name = \"t\"\n[server]\nkind = \"https\"\n[server.https]\nurl = \"https://127.0.0.1:{port}\"\ntls_pin = \"{CERT_PIN}\"\n"
    );
    let parsed = config::parse(
        &text,
        Path::new("/c.toml"),
        &MapEnv::new().with("HOME", "/home/t"),
    )
    .unwrap();
    let client = Client::new(
        parsed.server.https.as_ref().unwrap(),
        ApiKey::parse("pal_000000000000_0000").unwrap(),
    )
    .unwrap();
    match client.viewer().await.unwrap_err() {
        HttpsError::Transport {
            reason, retryable, ..
        } => {
            // Refused for its signature, not for its certificate, which
            // carries the right key.
            assert!(
                reason.to_ascii_lowercase().contains("signature"),
                "{reason}"
            );
            assert!(!reason.contains("not the configured tls_pin"), "{reason}");
            assert!(!retryable, "{reason}");
        }
        other => panic!("unexpected {other:?}"),
    }
}
