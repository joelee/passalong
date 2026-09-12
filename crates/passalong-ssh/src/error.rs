//! Errors from reaching the server over SSH and starting SFTP.

use passalong_core::store::StoreError;

/// SSH connection failures, each with a message a user can act on.
#[derive(Debug, thiserror::Error)]
pub enum SshError {
    /// `server.ssh.host_key` is not an OpenSSH public key.
    #[error("invalid server.ssh.host_key: {0}")]
    InvalidHostKey(String),
    /// The server presented a key other than the pinned one.
    #[error(
        "host key mismatch for {host}: the server presented {actual}, but server.ssh.host_key is {expected}. \
         If the server's key really changed, get the new key with `ssh-keyscan -t ed25519 {host}` \
         and update server.ssh.host_key"
    )]
    HostKeyMismatch {
        /// The configured host.
        host: String,
        /// Fingerprint of the pinned key.
        expected: String,
        /// Fingerprint of the key the server presented.
        actual: String,
    },
    /// The private key could not be read or decrypted.
    #[error("cannot load the SSH key {path}: {message}")]
    KeyLoad {
        /// Path of the key file.
        path: String,
        /// Why loading failed, such as a missing passphrase.
        message: String,
    },
    /// The TCP connection or SSH handshake failed.
    #[error("cannot connect to {address}: {message}")]
    Connect {
        /// `host:port`.
        address: String,
        /// Underlying error.
        message: String,
    },
    /// Connecting and authenticating took longer than `connect_timeout_secs`.
    #[error("connecting to {address} timed out after {secs} s")]
    Timeout {
        /// `host:port`.
        address: String,
        /// The configured timeout.
        secs: u64,
    },
    /// The server refused the key for this user.
    #[error("the server at {address} rejected the key {path} for user `{user}`")]
    AuthenticationFailed {
        /// `host:port`.
        address: String,
        /// Login user.
        user: String,
        /// Path of the key that was offered.
        path: String,
    },
    /// The SSH session opened but SFTP could not start or prepare the root.
    #[error("cannot start SFTP on {address}: {message}")]
    Sftp {
        /// `host:port`.
        address: String,
        /// Underlying error.
        message: String,
    },
}

impl From<SshError> for StoreError {
    fn from(err: SshError) -> Self {
        StoreError::Backend(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mismatch_message_explains_how_to_fix_it() {
        let err = SshError::HostKeyMismatch {
            host: "nas".into(),
            expected: "SHA256:aaa".into(),
            actual: "SHA256:bbb".into(),
        };
        let msg = err.to_string();
        assert!(msg.starts_with("host key mismatch for nas"), "{msg}");
        assert!(
            msg.contains("SHA256:bbb") && msg.contains("ssh-keyscan -t ed25519 nas"),
            "{msg}"
        );
        match StoreError::from(err) {
            StoreError::Backend(text) => assert_eq!(text, msg),
            other => panic!("unexpected {other:?}"),
        }
    }
}
