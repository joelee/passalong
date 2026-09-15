//! Tests against a real OpenSSH server. They are ignored by default; run
//! them with `just test-integration`, which starts the server in Docker and
//! exports the `PASSALONG_IT_SSH_*` variables. They fail, rather than skip,
//! when those variables are missing.

use std::io::Cursor;
use std::sync::Arc;

use passalong_core::clock::SystemClock;
use passalong_core::config::{self, SshConfig};
use passalong_core::crypto::{DataKey, KdfParams, Words};
use passalong_core::encryption;
use passalong_core::fs::{FsError, RemoteFs, RemotePath};
use passalong_core::model::NewItem;
use passalong_core::random::{RandomSource, StdRandom};
use passalong_core::store::{FsStore, Store, StoreError, WriteProbe};
use passalong_core::testing::{FaultyFs, FixedClock, FsOp, ManualClock, MapEnv};
use passalong_ssh::connect::SshParams;
use passalong_ssh::error::SshError;
use passalong_ssh::fetch_host_key;
use passalong_ssh::host_key::PinnedHostKey;
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
    let text = format!(
        "[server]\nkind = \"ssh\"\n\n[server.ssh]\nhost = \"{}\"\nport = {}\nuser = \"{}\"\nhost_key = \"{}\"\nidentity_file = \"{}\"\nremote_path = \"{}\"\nconnect_timeout_secs = 10\n",
        var("PASSALONG_IT_SSH_HOST"),
        var("PASSALONG_IT_SSH_PORT"),
        var("PASSALONG_IT_SSH_USER"),
        var("PASSALONG_IT_SSH_HOST_KEY"),
        var("PASSALONG_IT_SSH_IDENTITY"),
        var("PASSALONG_IT_SSH_REMOTE_PATH"),
    );
    let env = MapEnv::new().with("HOME", "/nonexistent-home");
    config::parse(&text, std::path::Path::new("/it.toml"), &env)
        .unwrap()
        .server
        .ssh
        .unwrap()
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
async fn delete_and_clean_staging_work_over_sftp() {
    let params = SshParams::from_config(&config()).unwrap();
    let fs = SftpFs::open(&params, &unique_root()).await.unwrap();
    // A clock one day ahead makes a freshly created staging directory "old".
    let tomorrow = (chrono::Utc::now() + chrono::TimeDelta::days(1)).to_rfc3339();
    let store = FsStore::new(
        fs,
        Arc::new(FixedClock::at(&tomorrow)),
        Box::new(StdRandom::new()),
    );
    let keep = store
        .put(NewItem::text("it"), Box::new(Cursor::new(b"keep".to_vec())))
        .await
        .unwrap()
        .meta;
    let gone = store
        .put(NewItem::text("it"), Box::new(Cursor::new(b"gone".to_vec())))
        .await
        .unwrap()
        .meta;
    assert_eq!(store.delete(&gone.id).await.unwrap(), gone);
    assert_eq!(store.list().await.unwrap(), vec![keep]);
    store
        .fs()
        .create_dir_all(&p("tmp/abandoned-upload"))
        .await
        .unwrap();
    assert_eq!(
        store
            .clean_staging(std::time::Duration::from_secs(3600))
            .await
            .unwrap(),
        1
    );
    assert!(
        store
            .fs()
            .stat(&p("tmp/abandoned-upload"))
            .await
            .unwrap()
            .is_none()
    );
    store
        .fs()
        .remove_dir_all(&RemotePath::root())
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "needs the Docker SSH server: just test-integration"]
async fn the_fetched_host_key_is_the_servers_ed25519_key() {
    let cfg = config();
    let found = fetch_host_key(&cfg.host, cfg.port, std::time::Duration::from_secs(10))
        .await
        .unwrap();
    let expected = PinnedHostKey::parse(&cfg.host_key).unwrap();
    assert_eq!(found.fingerprint, expected.fingerprint());
    assert_eq!(found.algorithm, "ssh-ed25519");
    assert_eq!(
        PinnedHostKey::parse(&found.openssh_line)
            .unwrap()
            .fingerprint(),
        expected.fingerprint()
    );
}

#[tokio::test]
#[ignore = "needs the Docker SSH server: just test-integration"]
async fn fetching_from_a_closed_port_is_a_connection_error() {
    let cfg = config();
    let err = fetch_host_key(&cfg.host, 1, std::time::Duration::from_secs(10))
        .await
        .unwrap_err();
    assert!(matches!(err, SshError::Connect { .. }), "{err:?}");
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

#[cfg(feature = "rsa")]
#[tokio::test]
#[ignore = "needs the Docker SSH server: just test-integration"]
async fn an_rsa_identity_logs_in_with_the_rsa_feature() {
    let mut cfg = config();
    cfg.identity_file = var("PASSALONG_IT_SSH_RSA_IDENTITY").into();
    let params = SshParams::from_config(&cfg).unwrap();
    let root = unique_root();
    let fs = SftpFs::open(&params, &root).await.unwrap();
    fs.create_dir_all(&p("rsa")).await.unwrap();
    assert!(fs.stat(&p("rsa")).await.unwrap().is_some());
}

#[tokio::test]
#[ignore = "needs the Docker SSH server: just test-integration"]
async fn list_after_works_over_sftp() {
    let params = SshParams::from_config(&config()).unwrap();
    let fs = SftpFs::open(&params, &unique_root()).await.unwrap();
    let clock = Arc::new(ManualClock::at("2026-09-13T08:00:00Z"));
    let store = FsStore::new(fs, clock.clone(), Box::new(StdRandom::new()));
    let first = store
        .put(
            NewItem::text("it"),
            Box::new(Cursor::new(b"first".to_vec())),
        )
        .await
        .unwrap()
        .meta;
    clock.advance(1);
    let second = store
        .put(
            NewItem::text("it"),
            Box::new(Cursor::new(b"second".to_vec())),
        )
        .await
        .unwrap()
        .meta;
    assert_eq!(
        store.list_after(Some(&first.id)).await.unwrap(),
        vec![second]
    );
    assert_eq!(store.list_after(None).await.unwrap().len(), 2);
}

#[tokio::test]
#[ignore = "needs the Docker SSH server: just test-integration"]
async fn get_meta_works_over_sftp() {
    let params = SshParams::from_config(&config()).unwrap();
    let fs = SftpFs::open(&params, &unique_root()).await.unwrap();
    let store = FsStore::new(fs, Arc::new(SystemClock), Box::new(StdRandom::new()));
    let meta = store
        .put(
            NewItem::text("it"),
            Box::new(Cursor::new(b"meta only".to_vec())),
        )
        .await
        .unwrap()
        .meta;
    assert_eq!(store.get_meta(&meta.id).await.unwrap(), meta);
}

#[tokio::test]
#[ignore = "needs the Docker SSH server: just test-integration"]
async fn probe_write_works_over_sftp_and_leaves_nothing() {
    let params = SshParams::from_config(&config()).unwrap();
    let fs = SftpFs::open(&params, &unique_root()).await.unwrap();
    let store = FsStore::new(fs, Arc::new(SystemClock), Box::new(StdRandom::new()));
    assert_eq!(store.probe_write().await.unwrap(), WriteProbe::Verified);
    assert!(store.fs().read_dir(&p("tmp")).await.unwrap().is_empty());
    assert!(store.list_ids().await.unwrap().is_empty());
}

const W1: &str = "abacus abdomen abdominal abide abiding ability";
const W2: &str = "zoom zoom zoom zoom zoom zoom";

fn words(text: &str) -> Words {
    Words::parse(text).unwrap()
}

fn quick() -> KdfParams {
    KdfParams {
        m_kib: 64,
        t: 1,
        p: 1,
        salt: [3; 16],
    }
}

async fn sftp(root: &str) -> SftpFs {
    SftpFs::open(&SshParams::from_config(&config()).unwrap(), root)
        .await
        .unwrap()
}

async fn sealed(fs: SftpFs, key: Option<&DataKey>) -> Result<Box<dyn Store>, StoreError> {
    encryption::open_with_key(
        fs,
        key.cloned(),
        Arc::new(SystemClock),
        Box::new(StdRandom::new()),
    )
    .await
}

fn text(body: &str) -> passalong_core::fs::BoxRead {
    Box::new(Cursor::new(body.as_bytes().to_vec()))
}

#[tokio::test]
#[ignore = "needs the Docker SSH server: just test-integration"]
async fn create_dir_is_exclusive_and_remove_file_works_over_sftp() {
    let fs = sftp(&unique_root()).await;
    fs.create_dir_all(&p("x")).await.unwrap();
    fs.create_dir(&p("x/lock")).await.unwrap();
    assert!(matches!(
        fs.create_dir(&p("x/lock")).await,
        Err(FsError::AlreadyExists(_))
    ));
    let mut w = fs.open_write(&p("x/f")).await.unwrap();
    w.write_all(b"gone soon").await.unwrap();
    w.shutdown().await.unwrap();
    fs.remove_file(&p("x/f")).await.unwrap();
    assert!(fs.stat(&p("x/f")).await.unwrap().is_none());
    fs.remove_file(&p("x/f")).await.unwrap();
}

#[tokio::test]
#[ignore = "needs the Docker SSH server: just test-integration"]
async fn an_encrypted_store_works_over_sftp() {
    let root = unique_root();
    let fs = sftp(&root).await;
    let key = encryption::set_up(&fs, &words(W1), quick()).await.unwrap();
    let store = sealed(sftp(&root).await, Some(&key)).await.unwrap();
    let first = store
        .put(NewItem::text("it"), text("sealed over sftp"))
        .await
        .unwrap()
        .meta;
    assert_eq!(store.list().await.unwrap(), vec![first.clone()]);
    let mut body = String::new();
    store
        .get(&first.id)
        .await
        .unwrap()
        .1
        .read_to_string(&mut body)
        .await
        .unwrap();
    assert_eq!(body, "sealed over sftp");

    // New words swap the header and keep the key: the open store goes on.
    encryption::change_words(&fs, &words(W1), &words(W2), quick())
        .await
        .unwrap();
    assert_eq!(
        encryption::join(&fs, &words(W2)).await.unwrap().key_id(),
        key.key_id()
    );
    store
        .put(NewItem::text("it"), text("after new words"))
        .await
        .unwrap();
    store.delete(&first.id).await.unwrap();
    assert_eq!(store.list().await.unwrap().len(), 1);

    assert!(matches!(
        sealed(sftp(&root).await, None).await,
        Err(StoreError::Encryption(_))
    ));
}

#[tokio::test]
#[ignore = "needs the Docker SSH server: just test-integration"]
async fn a_cut_migration_is_finished_and_a_cut_rotation_undone_over_sftp() {
    let root = unique_root();
    let fs = sftp(&root).await;
    let plain = FsStore::new(&fs, Arc::new(SystemClock), Box::new(StdRandom::new()));
    for body in ["one", "two"] {
        plain.put(NewItem::text("it"), text(body)).await.unwrap();
    }
    let clock = Arc::new(SystemClock);

    // Renames: the lock, the source, two items, then the header swap, which
    // fails.
    let faulty = FaultyFs::new(&fs);
    faulty.fail_nth(FsOp::Rename, 5);
    assert!(
        encryption::migrate(&faulty, &words(W1), quick(), clock.clone())
            .await
            .is_err()
    );
    let key = encryption::finish(&fs, &words(W1), None, None, clock.clone())
        .await
        .unwrap();
    let store = FsStore::sealed(
        &fs,
        Arc::new(SystemClock),
        Box::new(StdRandom::new()),
        passalong_core::crypto::Sealer::new(key.clone()),
    );
    assert_eq!(store.list().await.unwrap().len(), 2);

    // The same for a rotation: moving the old header aside fails.
    let faulty = FaultyFs::new(&fs);
    faulty.fail_nth(FsOp::Rename, 5);
    assert!(
        encryption::rotate(&faulty, &key, &words(W2), quick(), clock)
            .await
            .is_err()
    );
    encryption::undo(&fs).await.unwrap();
    let store = FsStore::sealed(
        &fs,
        Arc::new(SystemClock),
        Box::new(StdRandom::new()),
        passalong_core::crypto::Sealer::new(key.clone()),
    );
    assert_eq!(store.list().await.unwrap().len(), 2);
}

#[tokio::test]
#[ignore = "needs the Docker SSH server: just test-integration"]
async fn an_encryption_folder_without_its_header_is_never_plaintext_over_sftp() {
    let root = unique_root();
    // Opened while the store was still plaintext.
    let early = sealed(sftp(&root).await, None).await.unwrap();
    early
        .put(NewItem::text("it"), text("before"))
        .await
        .unwrap();
    let fs = sftp(&root).await;
    fs.create_dir_all(&RemotePath::new("encryption").unwrap())
        .await
        .unwrap();
    assert!(matches!(
        sealed(sftp(&root).await, None).await,
        Err(StoreError::Encryption(
            encryption::EncryptionError::HeaderMissing
        ))
    ));
    assert!(early.put(NewItem::text("it"), text("after")).await.is_err());
    let plain = FsStore::new(&fs, Arc::new(SystemClock), Box::new(StdRandom::new()));
    assert_eq!(plain.list().await.unwrap().len(), 1);
}
