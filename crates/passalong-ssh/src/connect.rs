//! Opening an authenticated SSH session to the configured server.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use passalong_core::config::{Passphrase, SshConfig};
use russh::client::{self, Handle};
use russh::keys::{PrivateKeyWithHashAlg, PublicKey, PublicKeyOrCertificate};

use crate::error::SshError;
use crate::host_key::{DiscoveredKey, PinnedHostKey, fingerprint};

/// How often an idle session sends a keepalive, so NAT and firewalls do not
/// drop the long-lived connection `serve` keeps open.
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);

/// Everything needed to connect, validated from `[server.ssh]`.
#[derive(Debug, Clone)]
pub struct SshParams {
    /// Server host name or address.
    pub host: String,
    /// Server port.
    pub port: u16,
    /// Login user.
    pub user: String,
    /// The pinned server key.
    pub host_key: PinnedHostKey,
    /// Private key used to log in.
    pub identity_file: PathBuf,
    /// Passphrase for `identity_file`; redacted in `Debug`.
    pub passphrase: Option<Passphrase>,
    /// Limit for connecting and authenticating.
    pub connect_timeout: Duration,
}

impl SshParams {
    /// Validates the SSH settings, including the pinned host key.
    ///
    /// # Errors
    ///
    /// [`SshError::InvalidHostKey`] when `host_key` cannot be parsed.
    pub fn from_config(config: &SshConfig) -> Result<Self, SshError> {
        Ok(Self {
            host: config.host.clone(),
            port: config.port,
            user: config.user.clone(),
            host_key: PinnedHostKey::parse(&config.host_key)?,
            identity_file: config.identity_file.clone(),
            passphrase: config.passphrase.clone(),
            connect_timeout: Duration::from_secs(config.connect_timeout_secs),
        })
    }

    /// `host:port`, with IPv6 hosts in brackets.
    pub fn address(&self) -> String {
        format_address(&self.host, self.port)
    }
}

/// An authenticated SSH connection. Dropping it closes the connection.
pub struct SshSession {
    handle: Handle<HostKeyCheck>,
    address: String,
}

impl SshSession {
    /// The underlying `russh` handle, for opening channels.
    pub fn handle(&self) -> &Handle<HostKeyCheck> {
        &self.handle
    }

    /// `host:port` of the server.
    pub fn address(&self) -> &str {
        &self.address
    }
}

/// `russh` handler that accepts only the pinned key and remembers what the
/// server presented, so a mismatch can be reported with both fingerprints.
pub struct HostKeyCheck {
    pinned: PinnedHostKey,
    presented: Arc<Mutex<Option<String>>>,
}

impl client::Handler for HostKeyCheck {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let key = server_key.public_key();
        let trusted = self.pinned.matches(&key);
        if !trusted {
            *self
                .presented
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(fingerprint(&key));
        }
        Ok(trusted)
    }
}

/// `host:port`, with IPv6 hosts in brackets.
pub(crate) fn format_address(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// Client settings shared by every connection: keepalives for long `serve`
/// sessions, and `russh`'s default algorithm preferences, which try Ed25519
/// host keys first (a unit test guards that order).
pub(crate) fn client_config() -> client::Config {
    client::Config {
        keepalive_interval: Some(KEEPALIVE_INTERVAL),
        ..client::Config::default()
    }
}

/// `russh` handler for discovery: records the key the server presents and
/// refuses it, so no authentication is ever attempted.
pub(crate) struct KeyRecorder {
    pub(crate) recorded: Arc<Mutex<Option<PublicKey>>>,
}

impl client::Handler for KeyRecorder {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        *self.recorded.lock().unwrap_or_else(PoisonError::into_inner) =
            Some(server_key.public_key());
        Ok(false)
    }
}

/// Fetches the host key `host:port` presents, without trusting it and
/// without logging in. `init` shows its fingerprint so the user can confirm
/// it before it is pinned.
///
/// # Errors
///
/// [`SshError::Timeout`] after `timeout`, and [`SshError::Connect`] when the
/// server cannot be reached or presents no key.
pub async fn fetch_host_key(
    host: &str,
    port: u16,
    timeout: Duration,
) -> Result<DiscoveredKey, SshError> {
    let address = format_address(host, port);
    let recorded = Arc::new(Mutex::new(None));
    let recorder = KeyRecorder {
        recorded: recorded.clone(),
    };
    let attempt = client::connect(Arc::new(client_config()), (host, port), recorder);
    let outcome = tokio::time::timeout(timeout, attempt)
        .await
        .map_err(|_| SshError::Timeout {
            address: address.clone(),
            secs: timeout.as_secs(),
        })?;
    match outcome {
        // The recorder always refuses, so a rejected key is the normal end.
        Err(russh::Error::UnknownKey) | Ok(_) => {}
        Err(err) => {
            return Err(SshError::Connect {
                address,
                message: err.to_string(),
            });
        }
    }
    let key = recorded
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
        .ok_or_else(|| SshError::Connect {
            address: address.clone(),
            message: "the server presented no host key".to_owned(),
        })?;
    tracing::debug!("fetched the host key of {address}");
    DiscoveredKey::from_key(&key)
}

/// Connects and authenticates with the identity file, all within
/// `connect_timeout`. The key is loaded first, so key problems are reported
/// without contacting the server.
///
/// # Errors
///
/// Every [`SshError`] variant except `InvalidHostKey` and `Sftp`.
pub async fn connect(params: &SshParams) -> Result<SshSession, SshError> {
    let address = params.address();
    tokio::time::timeout(
        params.connect_timeout,
        connect_and_authenticate(params, &address),
    )
    .await
    .map_err(|_| SshError::Timeout {
        address: address.clone(),
        secs: params.connect_timeout.as_secs(),
    })?
}

async fn connect_and_authenticate(
    params: &SshParams,
    address: &str,
) -> Result<SshSession, SshError> {
    let key_path = params.identity_file.display().to_string();
    let key = russh::keys::load_secret_key(
        &params.identity_file,
        params.passphrase.as_ref().map(Passphrase::expose),
    )
    .map_err(|err| SshError::KeyLoad {
        path: key_path.clone(),
        message: err.to_string(),
    })?;

    let presented = Arc::new(Mutex::new(None));
    let handler = HostKeyCheck {
        pinned: params.host_key.clone(),
        presented: presented.clone(),
    };
    let config = Arc::new(client_config());
    let connect_error = |err: russh::Error| SshError::Connect {
        address: address.to_owned(),
        message: err.to_string(),
    };
    let mut handle =
        match client::connect(config, (params.host.as_str(), params.port), handler).await {
            Ok(handle) => handle,
            Err(russh::Error::UnknownKey) => {
                let actual = presented
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take()
                    .unwrap_or_else(|| "an unknown key".to_owned());
                return Err(SshError::HostKeyMismatch {
                    host: params.host.clone(),
                    expected: params.host_key.fingerprint(),
                    actual,
                });
            }
            Err(err) => return Err(connect_error(err)),
        };

    let key = Arc::new(key);
    let hash_alg = if key.algorithm().is_rsa() {
        handle
            .best_supported_rsa_hash()
            .await
            .map_err(connect_error)?
            .flatten()
    } else {
        None
    };
    let auth = handle
        .authenticate_publickey(&params.user, PrivateKeyWithHashAlg::new(key, hash_alg))
        .await
        .map_err(connect_error)?;
    if !auth.success() {
        return Err(SshError::AuthenticationFailed {
            address: address.to_owned(),
            user: params.user.clone(),
            path: key_path,
        });
    }
    tracing::debug!(path = %key_path, "SSH session established with {address}");
    Ok(SshSession {
        handle,
        address: address.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::SshError;
    use crate::host_key::tests::KEY_A;
    use passalong_core::config::{Passphrase, SshConfig};
    use std::path::PathBuf;
    use std::time::Duration;

    fn config() -> SshConfig {
        SshConfig {
            host: "192.168.1.10".into(),
            port: 2222,
            user: "pa".into(),
            host_key: KEY_A.into(),
            identity_file: PathBuf::from("/keys/id_ed25519"),
            remote_path: "/srv/passalong".into(),
            connect_timeout_secs: 7,
            passphrase: Some(Passphrase::new("s3cret-value")),
        }
    }

    #[test]
    fn parameters_come_from_the_ssh_config() {
        let params = SshParams::from_config(&config()).unwrap();
        assert_eq!(params.host, "192.168.1.10");
        assert_eq!(params.port, 2222);
        assert_eq!(params.address(), "192.168.1.10:2222");
        assert_eq!(params.user, "pa");
        assert_eq!(params.identity_file, PathBuf::from("/keys/id_ed25519"));
        assert_eq!(params.connect_timeout, Duration::from_secs(7));
        assert_eq!(
            params.passphrase.as_ref().map(Passphrase::expose),
            Some("s3cret-value")
        );
        assert!(params.host_key.fingerprint().starts_with("SHA256:"));
    }

    #[test]
    fn debug_output_never_shows_the_passphrase() {
        let params = SshParams::from_config(&config()).unwrap();
        let debug = format!("{params:?}");
        assert!(!debug.contains("s3cret-value"), "{debug}");
        assert!(debug.contains("<redacted>"), "{debug}");
    }

    #[test]
    fn an_invalid_pinned_key_is_rejected_before_connecting() {
        let mut cfg = config();
        cfg.host_key = "not a key".into();
        assert!(matches!(
            SshParams::from_config(&cfg),
            Err(SshError::InvalidHostKey(_))
        ));
    }

    #[test]
    fn ipv6_hosts_are_bracketed_in_addresses() {
        let mut cfg = config();
        cfg.host = "fe80::1".into();
        assert_eq!(
            SshParams::from_config(&cfg).unwrap().address(),
            "[fe80::1]:2222"
        );
    }

    #[tokio::test]
    async fn key_problems_are_reported_before_any_connection() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut params = SshParams::from_config(&config()).unwrap();
        params.identity_file = dir.path().join("missing_key");
        match connect(&params).await {
            Err(SshError::KeyLoad { path, .. }) => assert!(path.ends_with("missing_key"), "{path}"),
            other => panic!("unexpected {:?}", other.err()),
        }
        let garbage = dir.path().join("garbage");
        std::fs::write(&garbage, "not a private key").unwrap();
        params.identity_file = garbage;
        assert!(matches!(
            connect(&params).await,
            Err(SshError::KeyLoad { .. })
        ));
    }

    #[tokio::test]
    async fn the_key_recorder_keeps_the_presented_key_and_refuses_it() {
        use russh::client::Handler as _;
        let key = russh::keys::PublicKey::from_openssh(KEY_A).unwrap();
        let recorded = Arc::new(Mutex::new(None));
        let mut recorder = KeyRecorder {
            recorded: recorded.clone(),
        };
        let trusted = recorder
            .check_server_key(&russh::keys::PublicKeyOrCertificate::from(key.clone()))
            .await
            .unwrap();
        assert!(!trusted, "discovery must never trust the key");
        assert_eq!(
            recorded
                .lock()
                .unwrap()
                .as_ref()
                .map(|k: &russh::keys::PublicKey| k.key_data().clone()),
            Some(key.key_data().clone())
        );
    }

    #[test]
    fn the_client_prefers_ed25519_host_keys_and_keeps_sessions_alive() {
        let config = client_config();
        assert_eq!(
            config.preferred.key.first(),
            Some(&russh::keys::Algorithm::Ed25519)
        );
        assert_eq!(config.keepalive_interval, Some(Duration::from_secs(30)));
    }

    #[test]
    fn addresses_bracket_ipv6_hosts() {
        assert_eq!(format_address("nas.local", 22), "nas.local:22");
        assert_eq!(format_address("::1", 2222), "[::1]:2222");
    }
}
