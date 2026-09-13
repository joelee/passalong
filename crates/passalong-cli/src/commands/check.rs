//! `passalong check`: test the configuration and the server.

use std::io::Write;
use std::path::PathBuf;

use async_trait::async_trait;
use passalong_core::config::Config;
use passalong_core::store::{BackendRegistry, Store, StoreError, WriteProbe};

/// Opens the store a config names; [`BackendRegistry`] in production.
#[async_trait]
pub trait Opener: Send + Sync {
    /// Connects to the configured backend.
    async fn open(&self, config: &Config) -> Result<Box<dyn Store>, StoreError>;
}

#[async_trait]
impl Opener for BackendRegistry {
    async fn open(&self, config: &Config) -> Result<Box<dyn Store>, StoreError> {
        BackendRegistry::open(self, config).await
    }
}

/// The checks, in the order they run.
const CHECKS: [&str; 4] = ["config", "server", "storage read", "storage write"];

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
/// accepts a connection, that the store can be listed, and that it accepts
/// writes, printing one line per check. The first failure marks the
/// remaining checks skipped and ends with an error.
pub async fn run(
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
    let store = match opener.open(&config).await {
        Ok(store) => store,
        Err(err) => return report.fail(&format!("{server}: {err}"), &err.to_string()),
    };
    report.line("ok", &server)?;
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
    match store.probe_write().await {
        Ok(WriteProbe::Verified) => report.line("ok", "wrote and removed a probe in tmp/")?,
        Ok(_) => report.line("n/a", "not supported by this backend")?,
        Err(err) => return report.fail(&err.to_string(), &err.to_string()),
    }
    tracing::debug!("every check passed");
    Ok(())
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

    /// Hands out one prepared result, as a backend would.
    struct FakeOpener(Mutex<Option<Result<Box<dyn Store>, StoreError>>>);

    impl FakeOpener {
        fn new(result: Result<Box<dyn Store>, StoreError>) -> Self {
            Self(Mutex::new(Some(result)))
        }
    }

    #[async_trait]
    impl Opener for FakeOpener {
        async fn open(&self, _config: &Config) -> Result<Box<dyn Store>, StoreError> {
            self.0.lock().unwrap().take().expect("opened once")
        }
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
        opened: Result<Box<dyn Store>, StoreError>,
    ) -> (anyhow::Result<()>, String) {
        let mut out = Vec::new();
        let result = run(config, &FakeOpener::new(opened), &mut out).await;
        (result, String::from_utf8(out).unwrap())
    }

    #[tokio::test]
    async fn a_healthy_store_passes_every_check() {
        let (_dir, store) = store(2).await;
        let (result, out) = check(local(), Ok(store)).await;
        result.unwrap();
        assert_eq!(
            out,
            "config         ok    /etc/passalong/config.toml\n\
             server         ok    local /srv/share\n\
             storage read   ok    2 items\n\
             storage write  ok    wrote and removed a probe in tmp/\n"
        );
    }

    #[tokio::test]
    async fn a_config_error_fails_config_and_skips_the_rest() {
        let (result, out) = check(
            Err(anyhow::anyhow!("no config file found")),
            Err(StoreError::Backend("never opened".into())),
        )
        .await;
        assert_eq!(
            out,
            "config         FAIL  no config file found\n\
             server         skip\n\
             storage read   skip\n\
             storage write  skip\n"
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
        )
        .await;
        assert!(
            out.contains("server         FAIL  local /srv/share: host key mismatch\n"),
            "{out}"
        );
        assert!(
            out.ends_with("storage read   skip\nstorage write  skip\n"),
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
        let (_dir, store) = store(1).await;
        store.fs().fail_next(FsOp::ReadDir, 1);
        let (result, out) = check(local(), Ok(store)).await;
        assert!(out.contains("storage read   FAIL  "), "{out}");
        assert!(out.ends_with("storage write  skip\n"), "{out}");
        assert!(
            result
                .unwrap_err()
                .to_string()
                .starts_with("check failed: storage read")
        );
    }

    #[tokio::test]
    async fn an_unwritable_store_fails_storage_write() {
        let (_dir, store) = store(1).await;
        store.fs().fail_next(FsOp::OpenWrite, 1);
        let (result, out) = check(local(), Ok(store)).await;
        assert!(out.contains("storage read   ok    1 item\n"), "{out}");
        assert!(out.contains("storage write  FAIL  "), "{out}");
        assert!(
            result
                .unwrap_err()
                .to_string()
                .starts_with("check failed: storage write")
        );
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
        let (result, out) = check(local(), Ok(Box::new(store))).await;
        result.unwrap();
        assert!(out.contains("storage read   ok    0 items\n"), "{out}");
        assert!(
            out.ends_with("storage write  n/a   not supported by this backend\n"),
            "{out}"
        );
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
