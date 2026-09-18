//! `passalong check`: test the configuration and the server.

use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use passalong_core::config::Config;
use passalong_core::encryption::{self, EncryptionError, StoreState, SystemGit, load_key_file};
use passalong_core::fs::RemoteFs;
use passalong_core::store::{BackendRegistry, PROBE_BYTES, Store, StoreError, WriteProbe};

use crate::daemon::Status;

/// Opens the store a config names; [`BackendRegistry`] in production.
#[async_trait]
pub trait Opener: Send + Sync {
    /// Connects to the configured backend.
    async fn open(&self, config: &Config) -> Result<Box<dyn Store>, StoreError>;

    /// Connects to the configured backend's filesystem.
    async fn open_fs(&self, config: &Config) -> Result<Box<dyn RemoteFs>, StoreError>;
}

#[async_trait]
impl Opener for BackendRegistry {
    async fn open(&self, config: &Config) -> Result<Box<dyn Store>, StoreError> {
        BackendRegistry::open(self, config).await
    }

    async fn open_fs(&self, config: &Config) -> Result<Box<dyn RemoteFs>, StoreError> {
        BackendRegistry::open_fs(self, config).await
    }
}

/// The checks, in the order they run.
const CHECKS: [&str; 5] = [
    "config",
    "server",
    "encryption",
    "storage read",
    "storage write",
];

/// Prints one aligned line per check, in order.
struct Report<'a> {
    out: &'a mut dyn Write,
    next: usize,
}

impl Report<'_> {
    fn line(&mut self, status: &str, detail: &str) -> std::io::Result<()> {
        let name = CHECKS[self.next];
        self.next += 1;
        let line = format!("{name:<15}{status:<6}{detail}");
        writeln!(self.out, "{}", line.trim_end())
    }

    /// Reports the current check as failed and the rest as skipped, then
    /// returns the error that ends the command.
    fn fail(&mut self, detail: &str, reason: &str) -> anyhow::Result<()> {
        let name = CHECKS[self.next];
        self.line("FAIL", detail)?;
        while self.next < CHECKS.len() {
            self.line("skip", "")?;
        }
        anyhow::bail!("check failed: {name}: {reason}")
    }
}

/// Checks, in order, that the config loaded, that the server it names
/// accepts a connection, that this device can use the store's encryption,
/// that the store can be listed, and that it accepts writes, printing one
/// line per check. The first failure marks the
/// remaining checks skipped and ends with an error. A last line always says
/// whether `serve` runs on this machine; it is informational and never
/// fails, so it is shown after a failure too.
pub async fn run(
    loaded: anyhow::Result<(PathBuf, Config)>,
    opener: &dyn Opener,
    serve: Result<Status, String>,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let result = run_checks(loaded, opener, out).await;
    let (status, detail) = match serve {
        Ok(Status::Running { pid: Some(pid), .. }) => ("ok", format!("running (pid {pid})")),
        Ok(Status::Running { pid: None, .. }) => ("ok", "running".to_owned()),
        Ok(Status::NotRunning) => ("off", "not running".to_owned()),
        Err(reason) => ("n/a", format!("cannot tell: {reason}")),
    };
    let line = format!("{:<15}{status:<6}{detail}", "serve");
    writeln!(out, "{}", line.trim_end())?;
    result
}

async fn run_checks(
    loaded: anyhow::Result<(PathBuf, Config)>,
    opener: &dyn Opener,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let mut report = Report { out, next: 0 };
    let (path, config) = match loaded {
        Ok(loaded) => loaded,
        Err(err) => {
            let reason = format!("{err:#}");
            return report.fail(&reason, &reason);
        }
    };
    report.line("ok", &path.display().to_string())?;
    let server = describe(&config);
    let fs = match opener.open_fs(&config).await {
        Ok(fs) => fs,
        Err(err) => return report.fail(&format!("{server}: {err}"), &err.to_string()),
    };
    report.line("ok", &server)?;
    match encryption_status(fs.as_ref(), &config).await {
        Ok((status, detail)) => report.line(status, &detail)?,
        Err(reason) => return report.fail(&reason, &reason),
    }
    let store = match opener.open(&config).await {
        Ok(store) => store,
        Err(err) => return report.fail(&err.to_string(), &err.to_string()),
    };
    match store.list_ids().await {
        Ok(ids) => {
            let n = ids.len();
            report.line(
                "ok",
                &format!("{n} {}", if n == 1 { "item" } else { "items" }),
            )?;
        }
        Err(err) => return report.fail(&err.to_string(), &err.to_string()),
    }
    let started = Instant::now();
    match store.probe_write().await {
        Ok(WriteProbe::Verified) => report.line("ok", &probe_line(started.elapsed()))?,
        Ok(_) => report.line("n/a", "not supported by this backend")?,
        Err(err) => return report.fail(&err.to_string(), &err.to_string()),
    }
    tracing::debug!("every check passed");
    Ok(())
}

fn refused(err: EncryptionError) -> Result<(&'static str, String), String> {
    Err(err.to_string())
}

/// The `encryption` line: `off`, `on (key …)` with any plaintext items a
/// fresh start left, or why this device cannot use the store.
async fn encryption_status(
    fs: &dyn RemoteFs,
    config: &Config,
) -> Result<(&'static str, String), String> {
    let key = match &config.client.key_file {
        Some(path) => load_key_file(path, &SystemGit::new()).map_err(|err| err.to_string())?,
        None => None,
    };
    match encryption::inspect(fs)
        .await
        .map_err(|err| err.to_string())?
    {
        StoreState::Plain { .. } if key.is_some() => refused(EncryptionError::KeyWithoutEncryption),
        StoreState::Plain { .. } => Ok(("off", "not encrypted".to_owned())),
        StoreState::Encrypted { key_id, plain_left } => match key {
            None => refused(EncryptionError::NoKey),
            Some(key) if key.key_id() != key_id => refused(EncryptionError::KeyMismatch {
                device: key.key_id().short(),
                store: key_id.short(),
            }),
            Some(_) => {
                let mut detail = format!("on (key {})", key_id.short());
                if plain_left > 0 {
                    let items = if plain_left == 1 {
                        "item remains"
                    } else {
                        "items remain"
                    };
                    detail.push_str(&format!(
                        "; {plain_left} unencrypted {items} from before encryption"
                    ));
                }
                let left = encryption::leftovers(fs)
                    .await
                    .map_err(|err| err.to_string())?;
                if !left.is_empty() {
                    detail.push_str(&format!("; {left}"));
                }
                Ok(("ok", detail))
            }
        },
        StoreState::Rewriting { started } => refused(EncryptionError::Rewriting { started }),
        StoreState::Broken => refused(EncryptionError::HeaderMissing),
        _ => Err("this store's state is not known to this version of passalong".to_owned()),
    }
}

/// The `storage write` line: how long the probe took, and the rate that
/// makes. With so few bytes the time is mostly network round trips.
fn probe_line(elapsed: Duration) -> String {
    let secs = elapsed.as_secs_f64();
    let ms = secs * 1000.0;
    let time = if ms < 10.0 {
        format!("{ms:.1} ms")
    } else {
        format!("{ms:.0} ms")
    };
    // A zero duration would divide by zero; a microsecond is below any
    // real probe.
    let rate = PROBE_BYTES as f64 / secs.max(1e-6);
    format!(
        "wrote and removed a {PROBE_BYTES}-byte probe in {time} ({})",
        human_rate(rate)
    )
}

/// Bytes per second in B/s, KiB/s, or MiB/s.
fn human_rate(bytes_per_sec: f64) -> String {
    const KIB: f64 = 1024.0;
    if bytes_per_sec < KIB {
        format!("{bytes_per_sec:.0} B/s")
    } else if bytes_per_sec < KIB * KIB {
        format!("{:.1} KiB/s", bytes_per_sec / KIB)
    } else {
        format!("{:.1} MiB/s", bytes_per_sec / (KIB * KIB))
    }
}

/// The server `config` names, as the `server` line shows it.
fn describe(config: &Config) -> String {
    let server = &config.server;
    match (server.kind.as_str(), &server.ssh, &server.local) {
        ("ssh", Some(ssh), _) => format!(
            "ssh {}@{}:{}, {}",
            ssh.user, ssh.host, ssh.port, ssh.remote_path
        ),
        ("local", _, Some(local)) => format!("local {}", local.path.display()),
        (kind, _, _) => kind.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::support::bytes;
    use passalong_core::config;
    use passalong_core::crypto::{DataKey, KDF_SALT_LEN, KdfParams, Words};
    use passalong_core::encryption::{open_with_key, save_key_file};
    use passalong_core::fs::{BoxRead, LocalFs};
    use passalong_core::model::{ContentKey, ItemId, ItemMeta, NewItem};
    use passalong_core::random::StdRandom;
    use passalong_core::store::{FsStore, PutOutcome};
    use passalong_core::testing::{FaultyFs, FsOp, ManualClock, MapEnv};
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    const KEY: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIF2M9DqIpW9GMebpvjNg+bobwAbQKRBqPVMatyvyI4gq";

    type Fs = Result<Box<dyn RemoteFs>, StoreError>;
    type Opened = Result<Box<dyn Store>, StoreError>;

    /// Hands out one prepared filesystem and one prepared store, as a
    /// backend would.
    struct FakeOpener {
        fs: Mutex<Option<Fs>>,
        store: Mutex<Option<Opened>>,
    }

    impl FakeOpener {
        fn new(fs: Fs, store: Opened) -> Self {
            Self {
                fs: Mutex::new(Some(fs)),
                store: Mutex::new(Some(store)),
            }
        }
    }

    #[async_trait]
    impl Opener for FakeOpener {
        async fn open(&self, _config: &Config) -> Opened {
            self.store.lock().unwrap().take().expect("opened once")
        }

        async fn open_fs(&self, _config: &Config) -> Fs {
            self.fs.lock().unwrap().take().expect("opened once")
        }
    }

    fn fs_of(dir: &TempDir) -> Fs {
        Ok(Box::new(LocalFs::new(dir.path())))
    }

    fn unused() -> StoreError {
        StoreError::Backend("unused".into())
    }

    fn parse(server: &str) -> Config {
        let text = format!("[client]\ndevice_name = \"t\"\n\n{server}");
        let env = MapEnv::new().with("HOME", "/home/t");
        config::parse(&text, Path::new("c.toml"), &env).unwrap()
    }

    fn local() -> anyhow::Result<(PathBuf, Config)> {
        Ok((
            PathBuf::from("/etc/passalong/config.toml"),
            parse("[server]\nkind = \"local\"\n\n[server.local]\npath = \"/srv/share\"\n"),
        ))
    }

    /// A local store in a temporary directory holding `items` items, over a
    /// filesystem whose calls tests can fail.
    async fn store(items: usize) -> (TempDir, Box<FsStore<FaultyFs<LocalFs>>>) {
        let dir = TempDir::new().unwrap();
        let clock = Arc::new(ManualClock::at("2026-09-12T09:53:11Z"));
        let store = FsStore::new(
            FaultyFs::new(LocalFs::new(dir.path())),
            clock.clone(),
            Box::new(StdRandom::new()),
        );
        for i in 0..items {
            store
                .put(NewItem::text("box"), bytes(format!("item {i}").as_bytes()))
                .await
                .unwrap();
            clock.advance(1);
        }
        (dir, Box::new(store))
    }

    async fn check(
        config: anyhow::Result<(PathBuf, Config)>,
        fs: Fs,
        opened: Opened,
    ) -> (anyhow::Result<()>, String) {
        let mut out = Vec::new();
        let opener = FakeOpener::new(fs, opened);
        let result = run(config, &opener, Ok(Status::NotRunning), &mut out).await;
        (result, String::from_utf8(out).unwrap())
    }

    #[tokio::test]
    async fn a_healthy_store_passes_every_check() {
        let (dir, store) = store(2).await;
        let (result, out) = check(local(), fs_of(&dir), Ok(store)).await;
        result.unwrap();
        assert!(
            out.starts_with(
                "config         ok    /etc/passalong/config.toml\n\
             server         ok    local /srv/share\n\
             encryption     off   not encrypted\n\
             storage read   ok    2 items\n\
             storage write  ok    wrote and removed a 128-byte probe in "
            ),
            "{out}"
        );
        assert!(
            out.ends_with("/s)\nserve          off   not running\n"),
            "{out}"
        );
    }

    #[test]
    fn the_probe_line_shows_the_time_and_the_rate() {
        use std::time::Duration;
        assert_eq!(
            probe_line(Duration::from_millis(184)),
            "wrote and removed a 128-byte probe in 184 ms (696 B/s)"
        );
        assert_eq!(
            probe_line(Duration::from_millis(2)),
            "wrote and removed a 128-byte probe in 2.0 ms (62.5 KiB/s)"
        );
        assert_eq!(
            probe_line(Duration::from_micros(100)),
            "wrote and removed a 128-byte probe in 0.1 ms (1.2 MiB/s)"
        );
        assert!(
            probe_line(Duration::ZERO)
                .starts_with("wrote and removed a 128-byte probe in 0.0 ms (")
        );
    }

    #[tokio::test]
    async fn a_config_error_fails_config_and_skips_the_rest() {
        let (result, out) = check(
            Err(anyhow::anyhow!("no config file found")),
            Err(StoreError::Backend("never opened".into())),
            Err(StoreError::Backend("never opened".into())),
        )
        .await;
        assert_eq!(
            out,
            "config         FAIL  no config file found\n\
             server         skip\n\
             encryption     skip\n\
             storage read   skip\n\
             storage write  skip\n\
             serve          off   not running\n"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.starts_with("check failed: config: no config file found"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn a_failed_connection_fails_server_and_skips_storage() {
        let (result, out) = check(
            local(),
            Err(StoreError::Backend("host key mismatch".into())),
            Err(unused()),
        )
        .await;
        assert!(
            out.contains("server         FAIL  local /srv/share: host key mismatch\n"),
            "{out}"
        );
        assert!(
            out.ends_with(
                "storage read   skip\nstorage write  skip\nserve          off   not running\n"
            ),
            "{out}"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("check failed: server: host key mismatch"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn an_unreadable_store_fails_storage_read() {
        let (dir, store) = store(1).await;
        store.fs().fail_next(FsOp::ReadDir, 1);
        let (result, out) = check(local(), fs_of(&dir), Ok(store)).await;
        assert!(out.contains("storage read   FAIL  "), "{out}");
        assert!(
            out.ends_with("storage write  skip\nserve          off   not running\n"),
            "{out}"
        );
        assert!(
            result
                .unwrap_err()
                .to_string()
                .starts_with("check failed: storage read")
        );
    }

    #[tokio::test]
    async fn an_unwritable_store_fails_storage_write() {
        let (dir, store) = store(1).await;
        store.fs().fail_next(FsOp::OpenWrite, 1);
        let (result, out) = check(local(), fs_of(&dir), Ok(store)).await;
        assert!(out.contains("storage read   ok    1 item\n"), "{out}");
        assert!(out.contains("storage write  FAIL  "), "{out}");
        assert!(
            result
                .unwrap_err()
                .to_string()
                .starts_with("check failed: storage write")
        );
    }

    #[tokio::test]
    async fn the_serve_line_reports_the_pid_lock_without_affecting_the_result() {
        for (serve, line) in [
            (
                Ok(Status::Running {
                    pid: Some(4242),
                    ready: true,
                }),
                "serve          ok    running (pid 4242)\n",
            ),
            (
                Ok(Status::Running {
                    pid: None,
                    ready: false,
                }),
                "serve          ok    running\n",
            ),
            (Ok(Status::NotRunning), "serve          off   not running\n"),
            (
                Err("pid file unreadable".to_owned()),
                "serve          n/a   cannot tell: pid file unreadable\n",
            ),
        ] {
            let (dir, store) = store(0).await;
            let mut out = Vec::new();
            let opener = FakeOpener::new(fs_of(&dir), Ok(store));
            run(local(), &opener, serve.clone(), &mut out)
                .await
                .unwrap();
            let out = String::from_utf8(out).unwrap();
            assert!(out.ends_with(line), "{out}");
            let mut out = Vec::new();
            let opener = FakeOpener::new(Err(unused()), Err(unused()));
            let failed = run(Err(anyhow::anyhow!("no config")), &opener, serve, &mut out).await;
            assert!(failed.is_err());
            assert!(String::from_utf8(out).unwrap().ends_with(line));
        }
    }

    /// A store relying on the trait's default `probe_write`.
    struct NoProbe(FsStore<LocalFs>);

    #[async_trait]
    impl Store for NoProbe {
        async fn put(&self, item: NewItem, content: BoxRead) -> Result<PutOutcome, StoreError> {
            self.0.put(item, content).await
        }
        async fn list(&self) -> Result<Vec<ItemMeta>, StoreError> {
            self.0.list().await
        }
        async fn list_after(&self, after: Option<&ItemId>) -> Result<Vec<ItemMeta>, StoreError> {
            self.0.list_after(after).await
        }
        async fn get(&self, id: &ItemId) -> Result<(ItemMeta, BoxRead), StoreError> {
            self.0.get(id).await
        }
        async fn exists(&self, id: &ItemId) -> Result<bool, StoreError> {
            self.0.exists(id).await
        }
        async fn find_by_content_key(
            &self,
            key: &ContentKey,
        ) -> Result<Option<ItemMeta>, StoreError> {
            self.0.find_by_content_key(key).await
        }
        async fn resolve(&self, input: &str) -> Result<ItemId, StoreError> {
            self.0.resolve(input).await
        }
        async fn delete(&self, id: &ItemId) -> Result<ItemMeta, StoreError> {
            self.0.delete(id).await
        }
        async fn clean_staging(
            &self,
            older_than: std::time::Duration,
        ) -> Result<usize, StoreError> {
            self.0.clean_staging(older_than).await
        }
    }

    #[tokio::test]
    async fn a_backend_without_a_probe_says_so_and_passes() {
        let dir = TempDir::new().unwrap();
        let store = NoProbe(FsStore::new(
            LocalFs::new(dir.path()),
            Arc::new(ManualClock::at("2026-09-12T09:53:11Z")),
            Box::new(StdRandom::new()),
        ));
        let (result, out) = check(local(), fs_of(&dir), Ok(Box::new(store))).await;
        result.unwrap();
        assert!(out.contains("storage read   ok    0 items\n"), "{out}");
        assert!(
            out.contains("storage write  n/a   not supported by this backend\n"),
            "{out}"
        );
    }

    const WORDS: &str = "zoom zoom zoom zoom zoom zoom";

    fn quick() -> KdfParams {
        KdfParams {
            m_kib: 64,
            t: 1,
            p: 1,
            salt: [6; KDF_SALT_LEN],
        }
    }

    /// The local config, with this device's key in `key_file`.
    fn keyed(key_file: &Path) -> anyhow::Result<(PathBuf, Config)> {
        let text = format!(
            "[client]\ndevice_name = \"t\"\nkey_file = '{}'\n\n[server]\nkind = \"local\"\n\n[server.local]\npath = \"/srv/share\"\n",
            key_file.display()
        );
        let env = MapEnv::new().with("HOME", "/home/t");
        Ok((
            PathBuf::from("/etc/passalong/config.toml"),
            config::parse(&text, Path::new("c.toml"), &env).unwrap(),
        ))
    }

    #[tokio::test]
    async fn the_encryption_line_names_the_key_and_the_plaintext_left() {
        let dir = TempDir::new().unwrap();
        let fs = LocalFs::new(dir.path());
        let plain = FsStore::new(
            fs.clone(),
            Arc::new(ManualClock::at("2026-09-12T09:53:11Z")),
            Box::new(StdRandom::new()),
        );
        plain
            .put(NewItem::text("box"), bytes(b"old"))
            .await
            .unwrap();
        let key = encryption::fresh_start(&fs, &Words::parse(WORDS).unwrap(), quick())
            .await
            .unwrap();
        // An upload passalong 0.2.0 left in the plaintext staging.
        std::fs::create_dir_all(dir.path().join("tmp/cut-short")).unwrap();
        let keys = TempDir::new().unwrap();
        let key_file = keys.path().join("store.key");
        save_key_file(&key_file, &key, &SystemGit::new()).unwrap();
        let store = open_with_key(
            fs,
            Some(key.clone()),
            Arc::new(ManualClock::at("2026-09-12T09:53:11Z")),
            Box::new(StdRandom::new()),
        )
        .await
        .unwrap();
        let (result, out) = check(keyed(&key_file), fs_of(&dir), Ok(store)).await;
        result.unwrap();
        assert!(
            out.contains(&format!(
                "encryption     ok    on (key {}); 1 unencrypted item remains from before encryption; 1 unencrypted leftover of cut-short uploads\n",
                key.key_id().short()
            )),
            "{out}"
        );
        assert!(out.contains("storage read   ok    0 items\n"), "{out}");
    }

    #[tokio::test]
    async fn the_encryption_line_fails_without_the_key_or_during_a_rewrite() {
        let dir = TempDir::new().unwrap();
        encryption::set_up(
            &LocalFs::new(dir.path()),
            &Words::parse(WORDS).unwrap(),
            quick(),
        )
        .await
        .unwrap();
        let (result, out) = check(local(), fs_of(&dir), Err(unused())).await;
        assert!(
            out.contains("encryption     FAIL  the store is encrypted and this device has no key"),
            "{out}"
        );
        assert!(out.contains("storage read   skip\n"), "{out}");
        assert!(result.unwrap_err().to_string().contains("encrypt --join"));

        let keys = TempDir::new().unwrap();
        let other = keys.path().join("store.key");
        save_key_file(&other, &DataKey::generate().unwrap(), &SystemGit::new()).unwrap();
        let (_, out) = check(keyed(&other), fs_of(&dir), Err(unused())).await;
        assert!(out.contains("is not the store's key"), "{out}");

        std::fs::create_dir(dir.path().join(".rewrite")).unwrap();
        let (_, out) = check(local(), fs_of(&dir), Err(unused())).await;
        assert!(out.contains("encrypt --recover"), "{out}");
    }

    #[tokio::test]
    async fn the_encryption_line_fails_for_a_key_with_a_plaintext_store() {
        let dir = TempDir::new().unwrap();
        let keys = TempDir::new().unwrap();
        let key_file = keys.path().join("store.key");
        save_key_file(&key_file, &DataKey::generate().unwrap(), &SystemGit::new()).unwrap();
        let (result, out) = check(keyed(&key_file), fs_of(&dir), Err(unused())).await;
        assert!(
            out.contains("encryption     FAIL  this device has an encryption key, but the store is not encrypted"),
            "{out}"
        );
        assert!(result.is_err());
    }

    #[test]
    fn ssh_servers_are_described_by_login_and_path() {
        let config = parse(&format!(
            "[server]\nkind = \"ssh\"\n\n[server.ssh]\nhost = \"nas\"\nport = 2222\nuser = \"pa\"\nhost_key = \"{KEY}\"\nidentity_file = \"/home/t/.ssh/id_ed25519\"\nremote_path = \"/srv/passalong\"\n"
        ));
        assert_eq!(describe(&config), "ssh pa@nas:2222, /srv/passalong");
        assert_eq!(describe(&local().unwrap().1), "local /srv/share");
    }
}
