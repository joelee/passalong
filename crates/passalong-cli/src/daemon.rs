//! Background `serve`: where its pid and log files live, the pid-file lock
//! that keeps a single copy running, and starting detached processes.
//!
//! The lock is an exclusive advisory lock held for the process's lifetime,
//! so a pid file left behind by a crash is harmless: nobody holds its lock.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use passalong_core::cache::CACHE_FILE;
use passalong_core::config::EnvProvider;

/// The platform, which decides the file locations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    /// Linux and other Unix-like systems using the XDG layout.
    Linux,
    /// macOS, using `~/Library`.
    Mac,
    /// Windows, using `%LOCALAPPDATA%`.
    Windows,
}

impl Os {
    /// The platform this binary was built for.
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Mac
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Linux
        }
    }
}

/// `serve`'s pid and log files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatePaths {
    /// Pid file, also the single-instance lock.
    pub pid: PathBuf,
    /// Log file for `serve --daemon`.
    pub log: PathBuf,
    /// The item list `serve` keeps for `list` and `choose`.
    pub cache: PathBuf,
}

fn non_empty(env: &dyn EnvProvider, key: &str) -> Option<String> {
    env.var(key).filter(|value| !value.is_empty())
}

impl StatePaths {
    /// Linux: `${XDG_STATE_HOME:-~/.local/state}/passalong/` holds
    /// `serve.pid`, `serve.log`, and `list-cache.json`. macOS:
    /// `~/Library/Application Support/passalong/` holds `serve.pid` and
    /// `list-cache.json`, and the log is `~/Library/Logs/passalong/serve.log`.
    /// Windows: `%LOCALAPPDATA%\passalong\` holds all three; without
    /// `LOCALAPPDATA`, which Windows always sets, the Linux rules apply.
    /// `None` without a home.
    pub fn resolve(env: &dyn EnvProvider, os: Os) -> Option<Self> {
        match os {
            Os::Windows => match non_empty(env, "LOCALAPPDATA") {
                Some(local) => {
                    let dir = PathBuf::from(local).join("passalong");
                    Some(Self {
                        pid: dir.join("serve.pid"),
                        log: dir.join("serve.log"),
                        cache: dir.join(CACHE_FILE),
                    })
                }
                None => Self::resolve(env, Os::Linux),
            },
            Os::Mac => {
                let home = PathBuf::from(non_empty(env, "HOME")?);
                let dir = home.join("Library/Application Support/passalong");
                Some(Self {
                    pid: dir.join("serve.pid"),
                    log: home.join("Library/Logs/passalong/serve.log"),
                    cache: dir.join(CACHE_FILE),
                })
            }
            Os::Linux => {
                let state = match non_empty(env, "XDG_STATE_HOME")
                    .map(PathBuf::from)
                    .filter(|p| p.is_absolute())
                {
                    Some(state) => state,
                    None => PathBuf::from(non_empty(env, "HOME")?).join(".local/state"),
                };
                let dir = state.join("passalong");
                Some(Self {
                    pid: dir.join("serve.pid"),
                    log: dir.join("serve.log"),
                    cache: dir.join(CACHE_FILE),
                })
            }
        }
    }
}

/// Whether a `serve` holds the pid lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Someone holds the lock.
    Running {
        /// The pid it recorded, if readable.
        pid: Option<u32>,
        /// Whether it finished starting up.
        ready: bool,
    },
    /// Nobody holds the lock.
    NotRunning,
}

/// Failure to take the pid lock.
#[derive(Debug)]
pub enum LockError {
    /// Another `serve` holds it.
    AlreadyRunning(Option<u32>),
    /// The file could not be opened or written.
    Io(io::Error),
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyRunning(Some(pid)) => write!(f, "serve is already running (pid {pid})"),
            Self::AlreadyRunning(None) => write!(f, "serve is already running"),
            Self::Io(err) => write!(f, "cannot use the pid file: {err}"),
        }
    }
}

impl std::error::Error for LockError {}

/// The held pid lock. Releasing it removes the pid file.
#[derive(Debug)]
pub struct PidLock {
    file: File,
    path: PathBuf,
}

impl PidLock {
    /// Takes the lock and records this process's pid.
    ///
    /// # Errors
    ///
    /// [`LockError::AlreadyRunning`] when another process holds it.
    pub fn acquire(path: &Path) -> Result<Self, LockError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(LockError::Io)?;
        }
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(LockError::Io)?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                return Err(LockError::AlreadyRunning(read_state(path).0));
            }
            Err(TryLockError::Error(err)) => return Err(LockError::Io(err)),
        }
        file.set_len(0).map_err(LockError::Io)?;
        writeln!(file, "{}", std::process::id()).map_err(LockError::Io)?;
        Ok(Self {
            file,
            path: path.to_path_buf(),
        })
    }

    /// Records that start-up finished, for `serve --daemon` to report.
    ///
    /// # Errors
    ///
    /// When the pid file cannot be written.
    pub fn mark_ready(&self) -> io::Result<()> {
        let mut file = &self.file;
        file.seek(SeekFrom::End(0))?;
        file.write_all(b"ready\n")
    }
}

impl Drop for PidLock {
    fn drop(&mut self) {
        // Still holding the lock, so no other process is using this file.
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Reports whether a `serve` holds the lock at `path`.
///
/// # Errors
///
/// When an existing pid file cannot be opened or locked for another reason.
pub fn status(path: &Path) -> io::Result<Status> {
    let file = match OpenOptions::new().read(true).write(true).open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Status::NotRunning),
        Err(err) => return Err(err),
    };
    match file.try_lock() {
        Ok(()) => Ok(Status::NotRunning),
        Err(TryLockError::WouldBlock) => {
            let (pid, ready) = read_state(path);
            Ok(Status::Running { pid, ready })
        }
        Err(TryLockError::Error(err)) => Err(err),
    }
}

fn read_state(path: &Path) -> (Option<u32>, bool) {
    std::fs::read_to_string(path)
        .map(|text| parse_state(&text))
        .unwrap_or((None, false))
}

/// Parses a pid file: the pid on the first line, then `ready` once started.
pub fn parse_state(text: &str) -> (Option<u32>, bool) {
    let mut lines = text.lines();
    let pid = lines.next().and_then(|line| line.trim().parse().ok());
    (pid, lines.any(|line| line.trim() == "ready"))
}

/// The last `lines` lines written to `log` after byte `offset`.
pub fn log_tail(log: &Path, offset: u64, lines: usize) -> String {
    let mut text = String::new();
    if let Ok(mut file) = File::open(log) {
        let _ = file.seek(SeekFrom::Start(offset));
        let _ = file.read_to_string(&mut text);
    }
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

/// A command for `exe` that runs detached from this terminal: in its own
/// process group, so Ctrl-C and terminal hang-ups do not reach it, and with
/// standard output discarded. Callers choose standard input and error.
#[cfg(unix)]
pub fn detached_command(exe: &Path) -> std::process::Command {
    use std::os::unix::process::CommandExt;
    let mut command = std::process::Command::new(exe);
    command.process_group(0).stdout(std::process::Stdio::null());
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    use passalong_core::testing::MapEnv;
    use tempfile::TempDir;

    #[test]
    fn linux_state_goes_to_xdg_state_home_or_local_state() {
        let xdg = MapEnv::new()
            .with("XDG_STATE_HOME", "/state")
            .with("HOME", "/home/u");
        assert_eq!(
            StatePaths::resolve(&xdg, Os::Linux),
            Some(StatePaths {
                pid: "/state/passalong/serve.pid".into(),
                log: "/state/passalong/serve.log".into(),
                cache: "/state/passalong/list-cache.json".into(),
            })
        );
        let home = MapEnv::new()
            .with("XDG_STATE_HOME", "relative")
            .with("HOME", "/home/u");
        assert_eq!(
            StatePaths::resolve(&home, Os::Linux).unwrap().pid,
            PathBuf::from("/home/u/.local/state/passalong/serve.pid")
        );
        assert_eq!(StatePaths::resolve(&MapEnv::new(), Os::Linux), None);
    }

    #[test]
    fn macos_state_follows_library_conventions() {
        let env = MapEnv::new()
            .with("HOME", "/Users/u")
            .with("XDG_STATE_HOME", "/ignored");
        assert_eq!(
            StatePaths::resolve(&env, Os::Mac),
            Some(StatePaths {
                pid: "/Users/u/Library/Application Support/passalong/serve.pid".into(),
                log: "/Users/u/Library/Logs/passalong/serve.log".into(),
                cache: "/Users/u/Library/Application Support/passalong/list-cache.json".into(),
            })
        );
        assert_eq!(StatePaths::resolve(&MapEnv::new(), Os::Mac), None);
    }

    #[test]
    fn windows_state_goes_to_localappdata_and_ignores_home() {
        let env = MapEnv::new()
            .with("LOCALAPPDATA", "/local")
            .with("XDG_STATE_HOME", "/state")
            .with("HOME", "/home/u");
        let dir = Path::new("/local").join("passalong");
        assert_eq!(
            StatePaths::resolve(&env, Os::Windows),
            Some(StatePaths {
                pid: dir.join("serve.pid"),
                log: dir.join("serve.log"),
                cache: dir.join("list-cache.json"),
            })
        );
        let unix_only = MapEnv::new().with("HOME", "/home/u");
        assert_eq!(
            StatePaths::resolve(&unix_only, Os::Windows),
            StatePaths::resolve(&unix_only, Os::Linux)
        );
    }

    #[test]
    fn only_one_holder_of_the_pid_lock() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state/serve.pid");
        assert_eq!(status(&path).unwrap(), Status::NotRunning, "no file yet");
        let lock = PidLock::acquire(&path).unwrap();
        let me = std::process::id();
        assert_eq!(
            status(&path).unwrap(),
            Status::Running {
                pid: Some(me),
                ready: false
            }
        );
        match PidLock::acquire(&path) {
            Err(err) => assert_eq!(
                err.to_string(),
                format!("serve is already running (pid {me})")
            ),
            Ok(_) => panic!("second lock acquired"),
        }
        lock.mark_ready().unwrap();
        assert_eq!(
            status(&path).unwrap(),
            Status::Running {
                pid: Some(me),
                ready: true
            }
        );
        drop(lock);
        assert!(!path.exists(), "pid file removed on release");
        assert_eq!(status(&path).unwrap(), Status::NotRunning);
    }

    #[test]
    fn a_stale_pid_file_is_taken_over() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("serve.pid");
        std::fs::write(&path, "99999999\nready\n").unwrap();
        assert_eq!(
            status(&path).unwrap(),
            Status::NotRunning,
            "nobody holds the lock"
        );
        let lock = PidLock::acquire(&path).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("{}\n", std::process::id())
        );
        drop(lock);
    }

    #[test]
    fn pid_file_contents_are_read_leniently() {
        assert_eq!(parse_state("123\nready\n"), (Some(123), true));
        assert_eq!(parse_state("123\n"), (Some(123), false));
        assert_eq!(parse_state("garbage"), (None, false));
        assert_eq!(parse_state(""), (None, false));
    }

    #[test]
    fn log_tail_keeps_the_last_lines_after_an_offset() {
        let dir = TempDir::new().unwrap();
        let log = dir.path().join("serve.log");
        std::fs::write(&log, "old run\n").unwrap();
        let offset = std::fs::metadata(&log).unwrap().len();
        let lines: String = (1..=12).map(|i| format!("line {i}\n")).collect();
        std::fs::OpenOptions::new()
            .append(true)
            .open(&log)
            .unwrap()
            .write_all(lines.as_bytes())
            .unwrap();
        let tail = log_tail(&log, offset, 3);
        assert_eq!(tail, "line 10\nline 11\nline 12");
        assert_eq!(log_tail(&dir.path().join("missing.log"), 0, 3), "");
    }
}
