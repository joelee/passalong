//! TLS to a passalong-server: the one public key a `tls_pin` names, or the
//! operating system's trust store. There is no setting that turns
//! verification off.
//!
//! A pin is the SHA-256 of the certificate's SubjectPublicKeyInfo. With a
//! pin, no authority, name, or date is checked, which is how a self-signed
//! server is reached; the handshake's signature still is, or the pin would
//! prove nothing.

use std::sync::Arc;

use passalong_core::config::TlsPin;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{
    CryptoProvider, WebPkiSupportedAlgorithms, verify_tls12_signature, verify_tls13_signature,
};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, DigitallySignedStruct, OtherError, SignatureScheme};
use sha2::{Digest, Sha256};

/// The `ring` provider, which the SSH backend uses too.
pub fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// The TLS configuration for a server with `pin`, or without one.
///
/// # Errors
///
/// When the system's trust store cannot be used.
pub fn client_config(pin: Option<TlsPin>) -> Result<rustls::ClientConfig, rustls::Error> {
    let provider = provider();
    let verifier: Arc<dyn ServerCertVerifier> = match pin {
        Some(pin) => Arc::new(PinnedVerifier::new(
            pin,
            provider.signature_verification_algorithms,
        )),
        None => Arc::new(rustls_platform_verifier::Verifier::new(provider.clone())?),
    };
    // `dangerous()` is how rustls installs any verifier of one's own, the
    // system's included; both verify fully.
    Ok(rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth())
}

/// Trusts the certificate whose public key the pin names, and nothing else.
#[derive(Debug)]
pub struct PinnedVerifier {
    pin: TlsPin,
    algorithms: WebPkiSupportedAlgorithms,
}

impl PinnedVerifier {
    /// A verifier for `pin` that checks handshake signatures with
    /// `algorithms`.
    pub fn new(pin: TlsPin, algorithms: WebPkiSupportedAlgorithms) -> Self {
        Self { pin, algorithms }
    }
}

/// The server presented a key other than the pinned one.
#[derive(Debug)]
pub struct PinMismatch {
    /// The pin of the key it presented.
    pub presented: Option<TlsPin>,
}

impl std::fmt::Display for PinMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.presented {
            Some(pin) => write!(
                f,
                "the server's certificate has the pin {pin}, not the configured tls_pin"
            ),
            None => write!(f, "the server's certificate cannot be read"),
        }
    }
}

impl std::error::Error for PinMismatch {}

impl ServerCertVerifier for PinnedVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let presented = spki_pin(end_entity).ok();
        if presented == Some(self.pin) {
            return Ok(ServerCertVerified::assertion());
        }
        Err(rustls::Error::InvalidCertificate(CertificateError::Other(
            OtherError(Arc::new(PinMismatch { presented })),
        )))
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

/// The pin of the certificate `der`: the SHA-256 of its
/// SubjectPublicKeyInfo, the seventh element of `tbsCertificate` when the
/// optional version comes first.
///
/// # Errors
///
/// A description when `der` is not a DER certificate.
pub fn spki_pin(der: &[u8]) -> Result<TlsPin, String> {
    let mut certificate = Der(der).sequence()?;
    let mut tbs = certificate.sequence()?;
    // [0] EXPLICIT version, present from X.509 v2 on.
    if tbs.0.first() == Some(&0xa0) {
        tbs.next()?;
    }
    // serialNumber, signature, issuer, validity, subject.
    for _ in 0..5 {
        tbs.next()?;
    }
    let (tag, _, whole) = tbs.next()?;
    if tag != 0x30 {
        return Err("no SubjectPublicKeyInfo where one belongs".to_owned());
    }
    Ok(TlsPin::from_digest(Sha256::digest(whole).into()))
}

/// Reads DER elements one after another.
struct Der<'a>(&'a [u8]);

impl<'a> Der<'a> {
    /// The next element: its tag, its content, and its whole encoding.
    fn next(&mut self) -> Result<(u8, &'a [u8], &'a [u8]), String> {
        let bytes = self.0;
        let (&tag, rest) = bytes.split_first().ok_or("the certificate ends early")?;
        if tag & 0x1f == 0x1f {
            return Err("a tag longer than one byte".to_owned());
        }
        let (&first, rest) = rest.split_first().ok_or("the certificate ends early")?;
        let (len, rest) = if first < 0x80 {
            (usize::from(first), rest)
        } else {
            let count = usize::from(first & 0x7f);
            if count == 0 || count > 4 || rest.len() < count {
                return Err("a length DER does not allow".to_owned());
            }
            let len = rest[..count]
                .iter()
                .fold(0_usize, |len, &byte| (len << 8) | usize::from(byte));
            (len, &rest[count..])
        };
        if rest.len() < len {
            return Err("the certificate ends early".to_owned());
        }
        let header = bytes.len() - rest.len();
        self.0 = &rest[len..];
        Ok((tag, &rest[..len], &bytes[..header + len]))
    }

    /// The next element, which must be a SEQUENCE, to read inside.
    fn sequence(&mut self) -> Result<Der<'a>, String> {
        match self.next()? {
            (0x30, content, _) => Ok(Der(content)),
            _ => Err("a SEQUENCE was expected".to_owned()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::pki_types::pem::PemObject;

    /// A self-signed certificate for `localhost` and `127.0.0.1`, as
    /// `passalong-server tls self-signed` made it, and the pin it printed.
    const CERT: &str = include_str!("../tests/fixtures/self-signed.pem");
    const CERT_PIN: &str = "sha256/yFchGu73Az8s4FnAfymiO33A6WswijiRgzkEfdjrHYE=";

    fn cert() -> CertificateDer<'static> {
        CertificateDer::from_pem_slice(CERT.as_bytes()).unwrap()
    }

    #[test]
    fn the_pin_of_a_certificate_is_the_one_the_server_prints() {
        assert_eq!(spki_pin(&cert()).unwrap().to_string(), CERT_PIN);
    }

    #[test]
    fn what_is_not_a_certificate_has_no_pin() {
        let der = cert().to_vec();
        for bad in [
            &[][..],
            &der[..10],
            &[0x30, 0x84, 0xff, 0xff, 0xff, 0xff],
            &[0x04, 0x00],
        ] {
            assert!(spki_pin(bad).is_err(), "{bad:?}");
        }
    }

    fn verifier(pin: &str) -> PinnedVerifier {
        PinnedVerifier::new(
            TlsPin::parse(pin).unwrap(),
            provider().signature_verification_algorithms,
        )
    }

    #[test]
    fn only_the_pinned_key_is_trusted() {
        let name = ServerName::try_from("localhost").unwrap();
        let now = UnixTime::now();
        assert!(
            verifier(CERT_PIN)
                .verify_server_cert(&cert(), &[], &name, &[], now)
                .is_ok()
        );
        let other = "sha256/Zmh6rfhivXdsj8GLjp+OIAiXFIVu4jOzkCpZHQ1fKSU=";
        let err = verifier(other)
            .verify_server_cert(&cert(), &[], &name, &[], now)
            .unwrap_err();
        assert!(err.to_string().contains(CERT_PIN), "{err}");
    }

    #[test]
    fn the_verifier_offers_the_provider_s_signature_schemes() {
        // That the pinned key must still sign the handshake is shown by a
        // real handshake in `tests/rogue_server.rs`.
        assert!(!verifier(CERT_PIN).supported_verify_schemes().is_empty());
    }

    #[test]
    fn both_kinds_of_configuration_build() {
        assert!(client_config(Some(TlsPin::parse(CERT_PIN).unwrap())).is_ok());
        assert!(client_config(None).is_ok());
    }
}
