//! The `ssh` storage backend: [`FsStore`] over [`SftpFs`].

use std::sync::Arc;

use passalong_core::clock::SystemClock;
use passalong_core::config::Config;
use passalong_core::random::StdRandom;
use passalong_core::store::{BackendFuture, BackendRegistry, FsStore, Store, StoreError};

use crate::connect::SshParams;
use crate::sftp_fs::SftpFs;

/// The `server.kind` this backend serves.
pub const KIND: &str = "ssh";

/// Adds the `ssh` backend to `registry`.
pub fn register(registry: &mut BackendRegistry) {
    registry.register(KIND, opener);
}

fn opener(config: &Config) -> BackendFuture<'_> {
    Box::pin(open_ssh_store(config))
}

/// Connects to the server in `[server.ssh]` and returns a store on it.
///
/// # Errors
///
/// [`StoreError::Config`] when `[server.ssh]` is missing and
/// [`StoreError::Backend`] for every connection problem, with a message
/// that says how to fix it.
pub async fn open_ssh_store(config: &Config) -> Result<Box<dyn Store>, StoreError> {
    let ssh = config
        .server
        .ssh
        .as_ref()
        .ok_or_else(|| StoreError::Config("the `server.ssh` section is missing".to_owned()))?;
    let params = SshParams::from_config(ssh)?;
    let fs = SftpFs::open(&params, &ssh.remote_path).await?;
    Ok(Box::new(FsStore::new(
        fs,
        Arc::new(SystemClock),
        Box::new(StdRandom::new()),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_key::tests::KEY_A;
    use passalong_core::config;
    use passalong_core::store::{BackendRegistry, StoreError};
    use passalong_core::testing::MapEnv;
    use std::path::Path;

    fn ssh_config(host_key: &str, identity: &str) -> Config {
        let text = format!(
            "[server]\nkind = \"ssh\"\n[server.ssh]\nhost = \"127.0.0.1\"\nport = 1\nuser = \"u\"\nhost_key = \"{host_key}\"\nidentity_file = \"{identity}\"\nremote_path = \"/r\"\n"
        );
        config::parse(
            &text,
            Path::new("/c.toml"),
            &MapEnv::new().with("HOME", "/home/u"),
        )
        .unwrap()
    }

    #[test]
    fn register_adds_the_ssh_kind() {
        let mut registry = BackendRegistry::with_builtin();
        register(&mut registry);
        assert_eq!(registry.kinds(), ["local", "ssh"]);
    }

    #[tokio::test]
    async fn configuration_problems_surface_before_any_connection() {
        let mut registry = BackendRegistry::with_builtin();
        register(&mut registry);
        let err = registry
            .open(&ssh_config("not a key", "/nonexistent/key"))
            .await
            .err()
            .unwrap();
        assert!(
            matches!(&err, StoreError::Backend(m) if m.contains("invalid server.ssh.host_key")),
            "{err:?}"
        );
        let err = registry
            .open(&ssh_config(KEY_A, "/nonexistent/key"))
            .await
            .err()
            .unwrap();
        assert!(
            matches!(&err, StoreError::Backend(m) if m.contains("cannot load the SSH key")),
            "{err:?}"
        );
        let mut cfg = ssh_config(KEY_A, "/nonexistent/key");
        cfg.server.ssh = None;
        assert!(matches!(
            open_ssh_store(&cfg).await.err().unwrap(),
            StoreError::Config(_)
        ));
    }
}
