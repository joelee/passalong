//! A passalong-server API key: `pal_<key id>_<secret>`, a bearer credential
//! for one workspace. It is kept in `server.https.api_key_file` under the
//! rules of the store's key file: written owner-only, refused when others
//! may read it or when it sits in a git work tree that does not ignore it.
//!
//! The key id is public: the server shows it, and the list cache's identity
//! uses it. The secret is never shown, logged, or kept anywhere but the key
//! file and the `Authorization` header.

use std::fmt;
use std::path::Path;

use zeroize::Zeroizing;

use crate::encryption::{GitCheck, KeyFileError, Secret, check_git, load_secret, save_secret};

/// What every API key starts with.
pub const PREFIX: &str = "pal_";

/// The API key file.
const API_KEY: Secret = Secret {
    setting: "server.https.api_key_file",
    what: "passalong API key file",
};

/// Why a text is not an API key. Messages never quote the key.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ApiKeyError {
    /// It does not start with [`PREFIX`].
    #[error("an API key starts with `{PREFIX}`")]
    Prefix,
    /// It is not `pal_<key id>_<secret>` with both parts present.
    #[error("an API key is `{PREFIX}<key id>_<secret>`")]
    Shape,
    /// It holds spaces or other characters no key has.
    #[error("an API key holds only letters, digits, `-`, and `_`")]
    Characters,
}

/// An API key. `Debug` shows its public id alone.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey {
    text: Zeroizing<String>,
    /// Where the secret starts in `text`.
    secret_at: usize,
}

impl ApiKey {
    /// Reads an API key, ignoring surrounding white space.
    ///
    /// # Errors
    ///
    /// [`ApiKeyError`] when `text` is not an API key.
    pub fn parse(text: &str) -> Result<Self, ApiKeyError> {
        let text = text.trim();
        let rest = text.strip_prefix(PREFIX).ok_or(ApiKeyError::Prefix)?;
        if !text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(ApiKeyError::Characters);
        }
        let (id, secret) = rest.split_once('_').ok_or(ApiKeyError::Shape)?;
        if id.is_empty() || secret.is_empty() {
            return Err(ApiKeyError::Shape);
        }
        Ok(Self {
            text: Zeroizing::new(text.to_owned()),
            secret_at: PREFIX.len() + id.len() + 1,
        })
    }

    /// The key's public id, as the server shows it.
    pub fn id(&self) -> &str {
        &self.text[PREFIX.len()..self.secret_at - 1]
    }

    /// The whole key, for the `Authorization: Bearer` header and nothing
    /// else.
    pub fn expose(&self) -> &str {
        &self.text
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ApiKey({PREFIX}{}_…)", self.id())
    }
}

/// Reads the API key file at `path`: `None` when there is none.
///
/// # Errors
///
/// As [`crate::encryption::load_key_file`], with
/// [`KeyFileError::Damaged`] for a file that holds no API key.
pub fn load_api_key(path: &Path, git: &dyn GitCheck) -> Result<Option<ApiKey>, KeyFileError> {
    let Some(text) = load_secret(path, git, API_KEY)? else {
        return Ok(None);
    };
    ApiKey::parse(&text)
        .map(Some)
        .map_err(|err| KeyFileError::Damaged {
            path: path.display().to_string(),
            what: API_KEY.what.to_owned(),
            reason: err.to_string(),
        })
}

/// Writes `key` to `path` owner-only, replacing any key there.
///
/// # Errors
///
/// As [`crate::encryption::save_key_file`].
pub fn save_api_key(path: &Path, key: &ApiKey, git: &dyn GitCheck) -> Result<(), KeyFileError> {
    let mut line = Zeroizing::new(key.expose().to_owned());
    line.push('\n');
    save_secret(path, line.as_bytes(), git, API_KEY)
}

/// Refuses, before anything is written, an API key file place inside a git
/// work tree that does not ignore it.
///
/// # Errors
///
/// [`KeyFileError::InGitWorkTree`] or [`KeyFileError::GitUnavailable`].
pub fn check_api_key_location(path: &Path, git: &dyn GitCheck) -> Result<(), KeyFileError> {
    check_git(path, git, API_KEY)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const KEY: &str = "pal_k7Qx2b_sEcReT-0123456789abcdef";

    /// Outside a git work tree git is never asked.
    struct NoGit;

    impl GitCheck for NoGit {
        fn is_ignored(&self, _repo: &Path, _path: &Path) -> std::io::Result<bool> {
            panic!("git was asked outside a work tree")
        }
    }

    #[test]
    fn a_key_parses_into_its_public_id_and_never_shows_its_secret() {
        let key = ApiKey::parse(&format!("  {KEY}\n")).unwrap();
        assert_eq!(key.id(), "k7Qx2b");
        assert_eq!(key.expose(), KEY);
        let shown = format!("{key:?}");
        assert_eq!(shown, "ApiKey(pal_k7Qx2b_…)");
        assert!(!shown.contains("sEcReT"));
    }

    #[test]
    fn what_is_not_a_key_is_refused_without_quoting_it() {
        for (text, err) in [
            ("k7Qx2b_S3CR3T", ApiKeyError::Prefix),
            ("pal_", ApiKeyError::Shape),
            ("pal_S3CR3T", ApiKeyError::Shape),
            ("pal__S3CR3T", ApiKeyError::Shape),
            ("pal_S3CR3T_", ApiKeyError::Shape),
            ("pal_id_S3C R3T", ApiKeyError::Characters),
            ("pal_id_S3CR3Té", ApiKeyError::Characters),
        ] {
            assert_eq!(ApiKey::parse(text), Err(err.clone()), "{text}");
            assert!(!err.to_string().contains("S3C"), "{err}");
        }
    }

    #[test]
    fn the_key_file_round_trips_and_names_its_setting() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("cfg").join("api.key");
        assert_eq!(load_api_key(&path, &NoGit).unwrap(), None);
        let key = ApiKey::parse(KEY).unwrap();
        save_api_key(&path, &key, &NoGit).unwrap();
        assert_eq!(load_api_key(&path, &NoGit).unwrap(), Some(key));

        std::fs::write(&path, "store key, not an API key\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let err = load_api_key(&path, &NoGit).unwrap_err();
        assert!(
            err.to_string()
                .contains("not a valid passalong API key file"),
            "{err}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_key_file_others_may_read_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("api.key");
        save_api_key(&path, &ApiKey::parse(KEY).unwrap(), &NoGit).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = load_api_key(&path, &NoGit).unwrap_err();
        assert!(matches!(err, KeyFileError::Permissions { .. }), "{err}");
        assert!(err.to_string().contains("chmod 600"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn a_key_file_in_a_work_tree_names_its_own_setting() {
        struct Tracked;
        impl GitCheck for Tracked {
            fn is_ignored(&self, _repo: &Path, _path: &Path) -> std::io::Result<bool> {
                Ok(false)
            }
        }
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        let err = check_api_key_location(&dir.path().join("api.key"), &Tracked).unwrap_err();
        assert!(matches!(err, KeyFileError::InGitWorkTree { .. }), "{err}");
        assert!(
            err.to_string()
                .contains("set server.https.api_key_file outside"),
            "{err}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_key_file_is_private_on_windows_and_refused_once_others_may_read_it() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("api.key");
        save_api_key(&path, &ApiKey::parse(KEY).unwrap(), &NoGit).unwrap();
        assert!(
            crate::owner_only::only_owner(&path).unwrap(),
            "{}",
            crate::owner_only::describe(&path)
        );
        let status = std::process::Command::new("icacls")
            .arg(&path)
            .args(["/grant", "*S-1-5-32-545:(R)"])
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        let err = load_api_key(&path, &NoGit).unwrap_err();
        assert!(matches!(err, KeyFileError::Permissions { .. }), "{err}");
        assert!(err.to_string().contains("icacls"), "{err}");
    }
}
