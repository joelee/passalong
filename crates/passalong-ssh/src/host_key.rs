//! Strict server host key pinning: the server's key must equal the
//! configured one. There is no trust-on-first-use and no `known_hosts`
//! fallback.

use russh::keys::{HashAlg, PublicKey};

use crate::error::SshError;

/// The server public key from `server.ssh.host_key`.
#[derive(Debug, Clone)]
pub struct PinnedHostKey {
    key: PublicKey,
}

impl PinnedHostKey {
    /// Parses an OpenSSH public key line such as `ssh-ed25519 AAAA… comment`.
    /// A leading host field, as printed by `ssh-keyscan`, is tolerated.
    ///
    /// # Errors
    ///
    /// [`SshError::InvalidHostKey`] when no public key can be read.
    pub fn parse(line: &str) -> Result<Self, SshError> {
        let line = line.trim();
        PublicKey::from_openssh(line)
            .or_else(|first| match line.split_once(char::is_whitespace) {
                Some((_, rest)) if !rest.trim().is_empty() => {
                    PublicKey::from_openssh(rest.trim()).map_err(|_| first)
                }
                _ => Err(first),
            })
            .map(|key| Self { key })
            .map_err(|err| {
                SshError::InvalidHostKey(format!(
                    "expected an OpenSSH public key such as `ssh-ed25519 AAAA...` ({err})"
                ))
            })
    }

    /// Whether `presented` is this key. Comments are ignored.
    pub fn matches(&self, presented: &PublicKey) -> bool {
        self.key.key_data() == presented.key_data()
    }

    /// The key's SHA-256 fingerprint, as `ssh-keygen -l` prints it.
    pub fn fingerprint(&self) -> String {
        fingerprint(&self.key)
    }
}

/// SHA-256 fingerprint of any public key.
pub(crate) fn fingerprint(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::error::SshError;

    // Throwaway public keys generated for these tests; public keys are not secrets.
    pub const KEY_A: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIF2M9DqIpW9GMebpvjNg+bobwAbQKRBqPVMatyvyI4gq passalong-test-a";
    pub const KEY_B: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIMKy9BQGg0B6NYvYwyrJzGCOHCXKQBj7E/5jvWJSEDCi passalong-test-b";
    pub const KEY_C: &str = "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBDjE+RqfPVU1q3oRZ0A7kEGVjiycIFrzmrvdTMY9XxA5RwiwmWJ1Ra9XgfC5jOjwYXCvbge0k6d0MwNVN8fH/Rg= passalong-test-c";
    const KEY_A_FINGERPRINT: &str = "SHA256:5Si4lWKPwa0+I2wCQf3eOtcF8jWo30BWybHoXLTxABo";

    fn public(line: &str) -> russh::keys::PublicKey {
        russh::keys::PublicKey::from_openssh(line).unwrap()
    }

    #[test]
    fn parses_openssh_key_lines_with_or_without_a_comment() {
        let pinned = PinnedHostKey::parse(KEY_A).unwrap();
        assert_eq!(pinned.fingerprint(), KEY_A_FINGERPRINT);
        let bare = KEY_A.rsplit_once(' ').unwrap().0;
        assert_eq!(
            PinnedHostKey::parse(bare).unwrap().fingerprint(),
            KEY_A_FINGERPRINT
        );
        assert_eq!(
            PinnedHostKey::parse(&format!("  {bare}\n"))
                .unwrap()
                .fingerprint(),
            KEY_A_FINGERPRINT
        );
    }

    #[test]
    fn accepts_ssh_keyscan_output_with_its_host_prefix() {
        let line = format!("[192.168.1.10]:2222 {KEY_A}");
        assert_eq!(
            PinnedHostKey::parse(&line).unwrap().fingerprint(),
            KEY_A_FINGERPRINT
        );
        let line = format!("192.168.1.10 {}", KEY_C);
        assert!(PinnedHostKey::parse(&line).is_ok());
    }

    #[test]
    fn matches_only_the_same_key_whatever_its_comment() {
        let pinned = PinnedHostKey::parse(KEY_A).unwrap();
        let recommented = KEY_A.replace("passalong-test-a", "someone@else");
        assert!(pinned.matches(&public(&recommented)));
        assert!(!pinned.matches(&public(KEY_B)), "different ed25519 key");
        assert!(!pinned.matches(&public(KEY_C)), "different key type");
    }

    #[test]
    fn rejects_anything_that_is_not_a_public_key() {
        for bad in [
            "",
            "   ",
            "ssh-ed25519",
            "ssh-ed25519 not*base64",
            "AAAAC3NzaC1lZDI1NTE5",
            "host ssh-ed25519",
            "a b c d",
        ] {
            match PinnedHostKey::parse(bad) {
                Err(SshError::InvalidHostKey(_)) => {}
                other => panic!("{bad:?}: {other:?}"),
            }
        }
    }
}
