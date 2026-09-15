//! The key file: where a device keeps its copy of an encrypted store's data
//! key.
//!
//! It is one line, `passalong-key 1 <key id> <data key>`, in lowercase hex.
//! It is written with mode 0600 through a temporary file and a rename, and
//! missing parent folders are created with mode 0700. A key file that group
//! or others can read is refused, as ssh refuses such private keys, and so
//! is one inside a git work tree that does not ignore it, so a key is never
//! committed by accident.

use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use zeroize::Zeroizing;

use crate::crypto::{DataKey, KEY_LEN, KeyId, random_bytes};

const MAGIC: &str = "passalong-key";
const FORMAT: &str = "1";

/// Why a key file cannot be read or written. Messages never contain the key.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum KeyFileError {
    /// Reading or writing failed.
    #[error("{path}: {message}")]
    Io {
        /// The key file.
        path: String,
        /// What went wrong.
        message: String,
    },
    /// Group or others can read the file.
    #[error("{path} can be read by other users; run `chmod 600 {path}`")]
    Permissions {
        /// The key file.
        path: String,
    },
    /// The file is in a git work tree that does not ignore it.
    #[error(
        "{path} is inside the git work tree {repo}, which does not ignore it; add it to .gitignore, or set client.key_file outside the repository"
    )]
    InGitWorkTree {
        /// The key file.
        path: String,
        /// The work tree.
        repo: String,
    },
    /// The file is in a git work tree, but git could not say whether it is
    /// ignored.
    #[error(
        "{path} is inside the git work tree {repo}, but git could not check that it is ignored ({reason}); set client.key_file outside the repository"
    )]
    GitUnavailable {
        /// The key file.
        path: String,
        /// The work tree.
        repo: String,
        /// Why git could not answer.
        reason: String,
    },
    /// The file is not a key file of this format.
    #[error("{path} is not a valid passalong key file: {reason}")]
    Damaged {
        /// The key file.
        path: String,
        /// What is wrong, never quoting the file.
        reason: String,
    },
}

/// Asks git whether it ignores a path.
pub trait GitCheck {
    /// Whether git, run in the work tree `repo`, ignores `path`.
    ///
    /// # Errors
    ///
    /// When git cannot be run or cannot answer.
    fn is_ignored(&self, repo: &Path, path: &Path) -> io::Result<bool>;
}

/// [`GitCheck`] that runs `git check-ignore`.
#[derive(Debug, Clone)]
pub struct SystemGit {
    program: OsString,
    envs: Vec<(OsString, OsString)>,
}

impl SystemGit {
    /// Runs `git` from `PATH`.
    pub fn new() -> Self {
        Self::with_program("git")
    }

    /// Runs `program` instead of `git`.
    pub fn with_program(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            envs: Vec::new(),
        }
    }

    /// Ignores the user's and the system's git configuration, so tests do
    /// not depend on global ignore rules.
    #[cfg(test)]
    fn isolated() -> Self {
        let mut git = Self::new();
        git.envs = vec![
            ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
            ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
        ];
        git
    }
}

impl Default for SystemGit {
    fn default() -> Self {
        Self::new()
    }
}

impl GitCheck for SystemGit {
    fn is_ignored(&self, repo: &Path, path: &Path) -> io::Result<bool> {
        let status = Command::new(&self.program)
            .arg("-C")
            .arg(repo)
            .args(["check-ignore", "-q", "--"])
            .arg(path)
            .envs(self.envs.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        match status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(io::Error::other(format!("git check-ignore {status}"))),
        }
    }
}

fn shown(path: &Path) -> String {
    path.display().to_string()
}

fn io_error(path: &Path, err: &io::Error) -> KeyFileError {
    KeyFileError::Io {
        path: shown(path),
        message: err.to_string(),
    }
}

/// The nearest folder above `path` that holds a `.git` entry.
fn work_tree(path: &Path) -> Option<PathBuf> {
    path.parent()?
        .ancestors()
        .find(|dir| dir.join(".git").exists())
        .map(Path::to_path_buf)
}

fn check_git(path: &Path, git: &dyn GitCheck) -> Result<(), KeyFileError> {
    let Some(repo) = work_tree(path) else {
        return Ok(());
    };
    match git.is_ignored(&repo, path) {
        Ok(true) => Ok(()),
        Ok(false) => Err(KeyFileError::InGitWorkTree {
            path: shown(path),
            repo: shown(&repo),
        }),
        Err(err) => Err(KeyFileError::GitUnavailable {
            path: shown(path),
            repo: shown(&repo),
            reason: err.to_string(),
        }),
    }
}

fn render(key: &DataKey) -> Zeroizing<String> {
    let mut line = Zeroizing::new(String::with_capacity(2 * KEY_LEN + 48));
    line.push_str(MAGIC);
    line.push(' ');
    line.push_str(FORMAT);
    line.push(' ');
    line.push_str(&key.key_id().to_string());
    line.push(' ');
    line.push_str(&Zeroizing::new(hex::encode(key.as_bytes())));
    line.push('\n');
    line
}

fn parse(text: &str) -> Result<DataKey, String> {
    let fields: Vec<&str> = text.split_whitespace().collect();
    let [magic, format, key_id, key] = fields[..] else {
        return Err(format!("expected 4 fields, found {}", fields.len()));
    };
    if magic != MAGIC {
        return Err("it does not start with `passalong-key`".to_owned());
    }
    if format != FORMAT {
        return Err(format!("format {format} is not supported"));
    }
    let key_id = KeyId::parse(key_id).map_err(|err| err.to_string())?;
    let bytes =
        Zeroizing::new(hex::decode(key).map_err(|_| "the key is not hexadecimal".to_owned())?);
    let bytes: [u8; KEY_LEN] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| format!("the key is not {KEY_LEN} bytes"))?;
    let key = DataKey::from_bytes(bytes);
    if key.key_id() != key_id {
        return Err("the key does not match its id".to_owned());
    }
    Ok(key)
}

/// Reads the key file at `path`: `None` when there is none.
///
/// # Errors
///
/// [`KeyFileError::Permissions`] when others can read it,
/// [`KeyFileError::InGitWorkTree`] or [`KeyFileError::GitUnavailable`] per
/// the git rule, [`KeyFileError::Damaged`] for a malformed file, and
/// [`KeyFileError::Io`] otherwise.
pub fn load_key_file(path: &Path, git: &dyn GitCheck) -> Result<Option<DataKey>, KeyFileError> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(io_error(path, &err)),
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(KeyFileError::Permissions { path: shown(path) });
        }
    }
    #[cfg(not(unix))]
    let _ = metadata;
    check_git(path, git)?;
    let text = Zeroizing::new(fs::read_to_string(path).map_err(|err| io_error(path, &err))?);
    parse(&text)
        .map(Some)
        .map_err(|reason| KeyFileError::Damaged {
            path: shown(path),
            reason,
        })
}

/// Writes `key` to `path`, replacing any key file there.
///
/// # Errors
///
/// [`KeyFileError::InGitWorkTree`] or [`KeyFileError::GitUnavailable`] per
/// the git rule, and [`KeyFileError::Io`] when writing fails; a failed write
/// leaves any earlier key file in place.
pub fn save_key_file(path: &Path, key: &DataKey, git: &dyn GitCheck) -> Result<(), KeyFileError> {
    check_git(path, git)?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    create_private_dirs(parent).map_err(|err| io_error(parent, &err))?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let token: [u8; 6] = random_bytes().map_err(|err| KeyFileError::Io {
        path: shown(path),
        message: err.to_string(),
    })?;
    let temp = parent.join(format!(".{name}.tmp-{}", hex::encode(token)));
    let written =
        write_private(&temp, render(key).as_bytes()).and_then(|()| fs::rename(&temp, path));
    if let Err(err) = written {
        let _ = fs::remove_file(&temp);
        return Err(io_error(path, &err));
    }
    Ok(())
}

fn create_private_dirs(dir: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    /// A git that must not be asked: outside a work tree it never is.
    struct NoGit;

    impl GitCheck for NoGit {
        fn is_ignored(&self, _repo: &Path, _path: &Path) -> io::Result<bool> {
            panic!("git was asked outside a work tree")
        }
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn a_saved_key_is_private_and_loads_back() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("new/folder/store.key");
        let key = DataKey::generate().unwrap();
        save_key_file(&path, &key, &NoGit).unwrap();

        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(&dir.path().join("new/folder")), 0o700);
        assert_eq!(mode(&dir.path().join("new")), 0o700);
        let text = fs::read_to_string(&path).unwrap();
        let fields: Vec<&str> = text.split_whitespace().collect();
        assert_eq!(
            fields[..3],
            ["passalong-key", "1", &key.key_id().to_string()[..]]
        );
        let loaded = load_key_file(&path, &NoGit).unwrap().unwrap();
        assert_eq!(loaded.as_bytes(), key.as_bytes());
    }

    #[test]
    fn saving_replaces_the_key_and_leaves_no_temporary_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("store.key");
        save_key_file(&path, &DataKey::generate().unwrap(), &NoGit).unwrap();
        let second = DataKey::generate().unwrap();
        save_key_file(&path, &second, &NoGit).unwrap();
        assert_eq!(
            load_key_file(&path, &NoGit).unwrap().unwrap().key_id(),
            second.key_id()
        );
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["store.key"]);
    }

    #[test]
    fn a_missing_key_file_is_none() {
        let dir = TempDir::new().unwrap();
        assert!(
            load_key_file(&dir.path().join("store.key"), &NoGit)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_key_file_others_can_read_is_refused() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("store.key");
        save_key_file(&path, &DataKey::generate().unwrap(), &NoGit).unwrap();
        for bits in [0o640, 0o604, 0o644] {
            fs::set_permissions(&path, fs::Permissions::from_mode(bits)).unwrap();
            let err = load_key_file(&path, &NoGit).unwrap_err();
            assert!(
                matches!(err, KeyFileError::Permissions { .. }),
                "{bits:o}: {err}"
            );
            assert!(err.to_string().contains("chmod 600"));
        }
    }

    #[test]
    fn a_damaged_key_file_is_refused_without_quoting_it() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("store.key");
        let key = DataKey::generate().unwrap();
        let good = render(&key);
        let key_hex = hex::encode(key.as_bytes());
        let other_id = DataKey::generate().unwrap().key_id().to_string();
        for bad in [
            String::new(),
            "passalong-key 1".to_owned(),
            good.replace("passalong-key", "other-key"),
            good.replacen(" 1 ", " 2 ", 1),
            good.replace(&key_hex, &"zz".repeat(KEY_LEN)),
            good.replace(&key_hex, &key_hex[..10]),
            good.replace(&key.key_id().to_string(), &other_id),
        ] {
            fs::write(&path, &bad).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            let err = load_key_file(&path, &NoGit).unwrap_err();
            assert!(
                matches!(err, KeyFileError::Damaged { .. }),
                "{bad:?}: {err}"
            );
            assert!(!err.to_string().contains(&key_hex[..16]), "{err}");
        }
    }

    fn git_init(dir: &Path) {
        let status = Command::new("git")
            .args(["init", "-q"])
            .arg(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .expect("git is needed for this test");
        assert!(status.success());
    }

    #[test]
    fn a_key_file_in_a_git_work_tree_is_refused_until_git_ignores_it() {
        let dir = TempDir::new().unwrap();
        git_init(dir.path());
        let path = dir.path().join("config/store.key");
        let key = DataKey::generate().unwrap();
        let git = SystemGit::isolated();

        let err = save_key_file(&path, &key, &git).unwrap_err();
        assert!(matches!(err, KeyFileError::InGitWorkTree { .. }), "{err}");
        assert!(!path.exists());

        fs::write(dir.path().join(".gitignore"), "*.key\n").unwrap();
        save_key_file(&path, &key, &git).unwrap();
        assert!(load_key_file(&path, &git).unwrap().is_some());

        fs::write(dir.path().join(".gitignore"), "").unwrap();
        let err = load_key_file(&path, &git).unwrap_err();
        assert!(matches!(err, KeyFileError::InGitWorkTree { .. }), "{err}");
    }

    #[test]
    fn a_work_tree_without_git_installed_is_refused() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();
        let git = SystemGit::with_program("passalong-test-no-such-git");
        let err = save_key_file(
            &dir.path().join("store.key"),
            &DataKey::generate().unwrap(),
            &git,
        )
        .unwrap_err();
        assert!(matches!(err, KeyFileError::GitUnavailable { .. }), "{err}");
    }
}
