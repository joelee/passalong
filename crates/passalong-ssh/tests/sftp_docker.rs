//! Tests against a real OpenSSH server. They are ignored by default; run
//! them with `just test-integration`, which starts the server in Docker and
//! exports the `PASSALONG_IT_SSH_*` variables. They fail, rather than skip,
//! when those variables are missing.

use std::io::Cursor;
use std::sync::Arc;

use passalong_core::clock::SystemClock;
use passalong_core::config::SshConfig;
use passalong_core::fs::{FsError, RemoteFs, RemotePath};
use passalong_core::model::NewItem;
use passalong_core::random::{RandomSource, StdRandom};
use passalong_core::store::{FsStore, Store};
use passalong_ssh::connect::SshParams;
use passalong_ssh::error::SshError;
use passalong_ssh::sftp_fs::SftpFs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A valid key that the test server does not have.
const OTHER_KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIMKy9BQGg0B6NYvYwyrJzGCOHCXKQBj7E/5jvWJSEDCi passalong-test-b";

fn var(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| {
        panic!("{name} is not set: run these tests with `just test-integration`")
    })
}

fn config() -> SshConfig {
    SshConfig {
        host: var("PASSALONG_IT_SSH_HOST"),
        port: var("PASSALONG_IT_SSH_PORT").parse().unwrap(),
        user: var("PASSALONG_IT_SSH_USER"),
        host_key: var("PASSALONG_IT_SSH_HOST_KEY"),
        identity_file: var("PASSALONG_IT_SSH_IDENTITY").into(),
        remote_path: var("PASSALONG_IT_SSH_REMOTE_PATH"),
        connect_timeout_secs: 10,
        passphrase: None,
    }
}

/// A fresh directory per test so tests can run in parallel.
fn unique_root() -> String {
    format!(
        "{}/it-{:016x}",
        var("PASSALONG_IT_SSH_REMOTE_PATH"),
        StdRandom::new().next_u64()
    )
}

fn p(path: &str) -> RemotePath {
    RemotePath::new(path).unwrap()
}

#[tokio::test]
#[ignore = "needs the Docker SSH server: just test-integration"]
async fn remote_fs_round_trip_over_sftp() {
    let params = SshParams::from_config(&config()).unwrap();
    let fs = SftpFs::open(&params, &unique_root()).await.unwrap();
    fs.create_dir_all(&p("a/b")).await.unwrap();
    fs.create_dir_all(&p("a/b")).await.unwrap();
    let mut w = fs.open_write(&p("a/b/f")).await.unwrap();
    w.write_all(b"over sftp").await.unwrap();
    w.shutdown().await.unwrap();
    let mut body = Vec::new();
    fs.open_read(&p("a/b/f"))
        .await
        .unwrap()
        .read_to_end(&mut body)
        .await
        .unwrap();
    assert_eq!(body, b"over sftp");
    let listed = fs.read_dir(&p("a")).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert!(listed[0].is_dir && listed[0].name == "b");
    let meta = fs.stat(&p("a/b/f")).await.unwrap().unwrap();
    assert_eq!((meta.is_dir, meta.size), (false, 9));
    assert!(fs.stat(&p("a/missing")).await.unwrap().is_none());
    assert!(matches!(
        fs.read_dir(&p("a/missing")).await.unwrap_err(),
        FsError::NotFound(_)
    ));

    fs.create_dir_all(&p("tmp/x")).await.unwrap();
    fs.create_dir_all(&p("items")).await.unwrap();
    fs.rename(&p("tmp/x"), &p("items/x")).await.unwrap();
    fs.create_dir_all(&p("tmp/y")).await.unwrap();
    assert!(matches!(
        fs.rename(&p("tmp/y"), &p("items/x")).await.unwrap_err(),
        FsError::AlreadyExists(_)
    ));

    fs.remove_dir_all(&RemotePath::root()).await.unwrap();
    assert!(fs.stat(&RemotePath::root()).await.unwrap().is_none());
    fs.remove_dir_all(&p("gone")).await.unwrap();
}

#[tokio::test]
#[ignore = "needs the Docker SSH server: just test-integration"]
async fn fs_store_works_over_sftp() {
    let params = SshParams::from_config(&config()).unwrap();
    let fs = SftpFs::open(&params, &unique_root()).await.unwrap();
    assert!(fs.root().starts_with('/'));
    let store = FsStore::new(fs, Arc::new(SystemClock), Box::new(StdRandom::new()));
    let data: Vec<u8> = (0..1024 * 1024_u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 7) as u8)
        .collect();
    let out = store
        .put(
            NewItem::file("big.bin", "it"),
            Box::new(Cursor::new(data.clone())),
        )
        .await
        .unwrap();
    assert!(out.created);
    assert_eq!(store.list().await.unwrap(), vec![out.meta.clone()]);
    let (_, mut stream) = store.get(&out.meta.id).await.unwrap();
    let mut body = Vec::new();
    stream.read_to_end(&mut body).await.unwrap();
    assert_eq!(body, data);
    let again = store
        .put(NewItem::file("big.bin", "it"), Box::new(Cursor::new(data)))
        .await
        .unwrap();
    assert!(!again.created);
    assert_eq!(
        store
            .resolve(&out.meta.id.content_key().as_str()[..6])
            .await
            .unwrap(),
        out.meta.id
    );
    store
        .fs()
        .remove_dir_all(&RemotePath::root())
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "needs the Docker SSH server: just test-integration"]
async fn a_different_host_key_is_refused() {
    let mut cfg = config();
    cfg.host_key = OTHER_KEY.to_owned();
    let params = SshParams::from_config(&cfg).unwrap();
    match SftpFs::open(&params, &unique_root()).await {
        Err(err @ SshError::HostKeyMismatch { .. }) => {
            assert!(err.to_string().contains("host key mismatch"))
        }
        other => panic!("unexpected {:?}", other.err()),
    }
}

#[tokio::test]
#[ignore = "needs the Docker SSH server: just test-integration"]
async fn an_unauthorised_identity_is_refused() {
    let dir = tempfile::TempDir::new().unwrap();
    let key = dir.path().join("stranger");
    let status = std::process::Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(&key)
        .status()
        .unwrap();
    assert!(status.success());
    let mut cfg = config();
    cfg.identity_file = key;
    let params = SshParams::from_config(&cfg).unwrap();
    assert!(matches!(
        SftpFs::open(&params, &unique_root()).await,
        Err(SshError::AuthenticationFailed { .. })
    ));
}

#[tokio::test]
#[ignore = "needs the Docker SSH server: just test-integration"]
async fn a_closed_port_is_a_connection_error() {
    let mut cfg = config();
    cfg.port = 1;
    let params = SshParams::from_config(&cfg).unwrap();
    assert!(matches!(
        SftpFs::open(&params, &unique_root()).await,
        Err(SshError::Connect { .. })
    ));
}
