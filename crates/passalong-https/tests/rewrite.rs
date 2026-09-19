//! Migrating and rotating a workspace through the server's rewrite
//! session, and finishing or undoing one that stopped, from the device
//! that began it or another. Ignored by default; `just test-https` runs
//! them.

mod support;

use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;

use passalong_core::api_key::{ApiKey, save_api_key};
use passalong_core::clock::SystemClock;
use passalong_core::crypto::{DataKey, KdfParams, Sealer, Words, wrap};
use passalong_core::encryption::{
    EncryptionAdmin, EncryptionError, RewriteKind, StoreHeader, StoreState, SystemGit,
};
use passalong_core::model::{ItemMeta, NewItem};
use passalong_core::store::{Store, StoreError};
use passalong_https::{Client, Code, HttpEncryptionAdmin, HttpStore, Partition};
use support::{Settings, SlowProxy, TestServer};
use tempfile::TempDir;
use tokio::io::AsyncReadExt;

const OLD: &str = "abacus abdomen abdominal abide abiding ability";
const NEW: &str = "zoom zoom zoom zoom zoom zoom";

fn words(text: &str) -> Words {
    Words::parse(text).unwrap()
}

fn quick() -> KdfParams {
    KdfParams {
        m_kib: 64,
        t: 1,
        p: 1,
        salt: [7; 16],
    }
}

/// A device of `server` in its own folder, with `key`.
struct Device {
    _dir: TempDir,
    client: Arc<Client>,
}

impl Device {
    fn new(server: &TestServer, key: &ApiKey) -> Self {
        Self::via(server, key, None)
    }

    fn via(server: &TestServer, key: &ApiKey, port: Option<u16>) -> Self {
        let dir = TempDir::new().unwrap();
        let config = match port {
            Some(port) => server.pinned_via(dir.path(), port),
            None => server.pinned(dir.path()),
        };
        let path = &config.server.https.as_ref().unwrap().api_key_file;
        save_api_key(path, key, &SystemGit::new()).unwrap();
        let client = passalong_https::connect(&config).unwrap();
        Self { _dir: dir, client }
    }

    fn admin(&self) -> HttpEncryptionAdmin {
        HttpEncryptionAdmin::new(self.client.clone(), Arc::new(SystemClock))
    }

    fn store(&self, key: Option<&DataKey>) -> HttpStore {
        HttpStore::new(
            self.client.clone(),
            key.cloned().map(Sealer::new),
            Arc::new(SystemClock),
        )
    }
}

async fn read_all(store: &dyn Store, meta: &ItemMeta) -> Vec<u8> {
    let (_, mut content) = store.get(&meta.id).await.unwrap();
    let mut bytes = Vec::new();
    content.read_to_end(&mut bytes).await.unwrap();
    bytes
}

/// Stores `n` items, a file among them, and returns them with their
/// contents.
async fn seed(store: &dyn Store, n: usize) -> Vec<(ItemMeta, Vec<u8>)> {
    let mut items = Vec::new();
    for i in 0..n {
        let bytes = format!("item {i}: {}", "x".repeat(i * 1000)).into_bytes();
        let item = if i == 1 {
            NewItem::file(format!("file-{i}.txt"), "box")
        } else {
            NewItem::text("box")
        };
        let meta = store
            .put(item, Box::new(Cursor::new(bytes.clone())))
            .await
            .unwrap()
            .meta;
        items.push((meta, bytes));
    }
    items
}

/// Checks that `store` holds exactly `items`, by what they were, not by
/// id: every one there, readable, and nothing else.
async fn assert_holds(store: &dyn Store, items: &[(ItemMeta, Vec<u8>)]) {
    let listed = store.list().await.unwrap();
    assert_eq!(listed.len(), items.len(), "{listed:?}");
    for (before, bytes) in items {
        let now = listed
            .iter()
            .find(|meta| meta.sha256 == before.sha256)
            .unwrap_or_else(|| panic!("{} is gone", before.id));
        assert_eq!(
            (now.created_at, &now.kind, &now.name, now.size),
            (before.created_at, &before.kind, &before.name, before.size)
        );
        assert_eq!(&read_all(store, now).await, bytes);
    }
}

/// Opens a migration by hand, as a device that then stopped would have:
/// `beginRewrite` with a header `words` unlock, and the first `staged`
/// items copied. Returns the new key.
async fn stopped_migration(
    device: &Device,
    items: &[(ItemMeta, Vec<u8>)],
    staged: usize,
) -> DataKey {
    let key = DataKey::generate().unwrap();
    let header = StoreHeader::new(wrap(&key, &words(NEW), quick()).unwrap());
    let header = String::from_utf8(header.to_json()).unwrap();
    let body = format!(
        "{{\"kind\":\"migrate\",\"expectedKeyId\":null,\"newKeyId\":\"{}\",\"newHeader\":{}}}",
        key.key_id(),
        header.trim_end()
    );
    post(&device.client, "/rewrite", body).await.unwrap();
    let target = HttpStore::in_partition(
        device.client.clone(),
        Some(Sealer::new(key.clone())),
        Arc::new(SystemClock),
        Partition::Staged,
    );
    for (meta, bytes) in &items[..staged] {
        target
            .import(meta, Box::new(Cursor::new(bytes.clone())))
            .await
            .unwrap();
    }
    key
}

async fn post(
    client: &Client,
    path: &str,
    body: String,
) -> Result<reqwest::Response, passalong_https::HttpsError> {
    let url = client.url(path);
    client
        .send("test", |http| {
            http.post(&url)
                .header("content-type", "application/json")
                .body(body.clone())
        })
        .await
}

#[tokio::test]
#[ignore = "needs passalong-server: just test-https"]
async fn a_migration_and_a_rotation_keep_every_item() {
    let server = TestServer::start();
    let device = Device::new(&server, &server.key);
    let items = seed(&device.store(None), 4).await;
    let admin = device.admin();

    let key = admin
        .migrate(&words(OLD), quick(), Arc::new(SystemClock))
        .await
        .unwrap();
    assert_eq!(
        admin.inspect().await.unwrap(),
        StoreState::Encrypted {
            key_id: key.key_id(),
            plain_left: 0
        }
    );
    assert_holds(&device.store(Some(&key)), &items).await;
    assert_eq!(
        admin.join(&words(OLD)).await.unwrap().key_id(),
        key.key_id()
    );

    let rotated = admin
        .rotate(&key, &words(NEW), quick(), Arc::new(SystemClock))
        .await
        .unwrap();
    assert_ne!(rotated.key_id(), key.key_id());
    assert_holds(&device.store(Some(&rotated)), &items).await;
    assert!(
        admin.join(&words(OLD)).await.is_err(),
        "the old words are done"
    );
    assert!(admin.open_rewrite().await.unwrap().is_none());

    // A rotation from a key that is no longer the store's is refused.
    match admin
        .rotate(&key, &words(OLD), quick(), Arc::new(SystemClock))
        .await
    {
        Err(StoreError::Encryption(EncryptionError::KeyMismatch { .. })) => {}
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
#[ignore = "needs passalong-server: just test-https"]
async fn a_stopped_migration_is_finished_by_its_device_after_each_stage() {
    for staged in 0..=3 {
        let server = TestServer::start();
        let device = Device::new(&server, &server.key);
        let items = seed(&device.store(None), 3).await;
        let admin = device.admin();
        let key = stopped_migration(&device, &items, staged).await;
        let open = admin.open_rewrite().await.unwrap().unwrap();
        assert_eq!(
            (
                open.kind,
                open.mine,
                open.new_key,
                open.old_key,
                open.staged,
                open.items
            ),
            (
                RewriteKind::Migrate,
                true,
                key.key_id(),
                None,
                staged,
                items.len()
            )
        );
        match device
            .store(None)
            .put(
                NewItem::text("box"),
                Box::new(Cursor::new(b"late".to_vec())),
            )
            .await
        {
            Err(StoreError::Encryption(EncryptionError::Rewriting { .. })) => {}
            other => panic!("unexpected {other:?}"),
        }
        assert!(
            admin.resume_rewrite(&words(OLD), None, None).await.is_err(),
            "wrong words"
        );
        let finished = admin.resume_rewrite(&words(NEW), None, None).await.unwrap();
        assert_eq!(finished.key_id(), key.key_id());
        assert_holds(&device.store(Some(&key)), &items).await;
        assert!(admin.open_rewrite().await.unwrap().is_none());
    }
}

#[tokio::test]
#[ignore = "needs passalong-server: just test-https"]
async fn another_device_takes_over_once_the_lease_ends_and_finishes_or_undoes() {
    for finish in [true, false] {
        let server = TestServer::start_with(Settings {
            lease_secs: 1,
            ..Settings::default()
        });
        let first = Device::new(&server, &server.key);
        let items = seed(&first.store(None), 3).await;
        let key = stopped_migration(&first, &items, 1).await;

        let second = Device::new(&server, &server.create_key(&[]));
        let admin = second.admin();
        let open = admin.open_rewrite().await.unwrap().unwrap();
        assert!(!open.mine);
        assert_eq!(open.holder, server.key.id());
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert!(
            admin
                .open_rewrite()
                .await
                .unwrap()
                .unwrap()
                .lease_ended(chrono::Utc::now())
        );

        if finish {
            let finished = admin.resume_rewrite(&words(NEW), None, None).await.unwrap();
            assert_eq!(finished.key_id(), key.key_id());
            assert_holds(&second.store(Some(&key)), &items).await;
        } else {
            admin.abort_rewrite().await.unwrap();
            assert_eq!(
                admin.inspect().await.unwrap(),
                StoreState::Plain { items: items.len() }
            );
            assert_holds(&second.store(None), &items).await;
        }
        // The first device is no longer the holder.
        let heartbeat = post(&first.client, "/rewrite/heartbeat", String::new()).await;
        assert!(heartbeat.is_err(), "{heartbeat:?}");
    }
}

#[tokio::test]
#[ignore = "needs passalong-server: just test-https"]
async fn a_live_lease_keeps_other_devices_out() {
    let server = TestServer::start();
    let first = Device::new(&server, &server.key);
    let items = seed(&first.store(None), 2).await;
    stopped_migration(&first, &items, 1).await;
    let second = Device::new(&server, &server.create_key(&[]));
    let err = second
        .admin()
        .resume_rewrite(&words(NEW), None, None)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("LEASE_HELD"), "{err}");
    let err = second.admin().abort_rewrite().await.unwrap_err();
    assert!(err.to_string().contains("LEASE_HELD"), "{err}");
}

#[tokio::test]
#[ignore = "needs passalong-server: just test-https"]
async fn a_begin_replayed_after_an_abort_is_refused_and_a_new_one_succeeds() {
    let server = TestServer::start();
    let device = Device::new(&server, &server.key);
    let items = seed(&device.store(None), 2).await;
    let key = DataKey::generate().unwrap();
    let header = StoreHeader::new(wrap(&key, &words(NEW), quick()).unwrap());
    let header = String::from_utf8(header.to_json()).unwrap();
    let begin = format!(
        "{{\"kind\":\"migrate\",\"expectedKeyId\":null,\"newKeyId\":\"{}\",\"newHeader\":{}}}",
        key.key_id(),
        header.trim_end()
    );
    post(&device.client, "/rewrite", begin.clone())
        .await
        .unwrap();
    device.admin().abort_rewrite().await.unwrap();
    let err = post(&device.client, "/rewrite", begin).await.unwrap_err();
    assert_eq!(err.code(), Some(Code::RewriteEnded), "{err}");

    let key = device
        .admin()
        .migrate(&words(OLD), quick(), Arc::new(SystemClock))
        .await
        .unwrap();
    assert_holds(&device.store(Some(&key)), &items).await;
}

#[tokio::test]
#[ignore = "needs passalong-server: just test-https"]
async fn heartbeats_keep_the_lease_through_a_slow_rewrite() {
    let server = TestServer::start_with(Settings {
        // The server keeps whole seconds: one could end at once.
        lease_secs: 2,
        ..Settings::default()
    });
    let fast = Device::new(&server, &server.key);
    let items = seed(&fast.store(None), 8).await;
    let proxy = SlowProxy::start(server.port(), Duration::from_millis(80)).await;
    let slow = Device::via(&server, &server.key, Some(proxy.port));
    let rival = Device::new(&server, &server.create_key(&[]));

    let started = std::time::Instant::now();
    let migrating = tokio::spawn({
        let admin = slow.admin();
        async move {
            admin
                .migrate(&words(OLD), quick(), Arc::new(SystemClock))
                .await
        }
    });
    // Another device tries to take over all along; the lease never ends.
    let mut refused = 0;
    while !migrating.is_finished() {
        if rival.admin().open_rewrite().await.unwrap().is_some() {
            match post(&rival.client, "/rewrite/take-over", String::new()).await {
                // The rewrite committed between the two calls.
                Err(err) if err.code() == Some(Code::NotFound) => break,
                Err(err) => {
                    assert_eq!(err.code(), Some(Code::LeaseHeld), "{err}");
                    refused += 1;
                }
                Ok(_) => panic!("the lease ended during the rewrite"),
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let key = migrating.await.unwrap().unwrap();
    assert!(
        started.elapsed() > Duration::from_secs(4),
        "the run was not slower than the lease: {:?}",
        started.elapsed()
    );
    // Each refusal takes a while: the client waits out `LEASE_HELD` before
    // it gives up.
    assert!(refused >= 1, "{refused}");
    assert_holds(&fast.store(Some(&key)), &items).await;
}

#[tokio::test]
#[ignore = "needs passalong-server: just test-https"]
async fn a_stopped_rotation_is_finished_with_the_old_words() {
    let server = TestServer::start();
    let device = Device::new(&server, &server.key);
    let admin = device.admin();
    let old = admin.set_up(&words(OLD), quick()).await.unwrap();
    let items = seed(&device.store(Some(&old)), 3).await;

    // Begun, one item copied, then stopped.
    let new = DataKey::generate().unwrap();
    let header = StoreHeader::new(wrap(&new, &words(NEW), quick()).unwrap());
    let header = String::from_utf8(header.to_json()).unwrap();
    post(
        &device.client,
        "/rewrite",
        format!(
            "{{\"kind\":\"rotate\",\"expectedKeyId\":\"{}\",\"newKeyId\":\"{}\",\"newHeader\":{}}}",
            old.key_id(),
            new.key_id(),
            header.trim_end()
        ),
    )
    .await
    .unwrap();
    let staged = HttpStore::in_partition(
        device.client.clone(),
        Some(Sealer::new(new.clone())),
        Arc::new(SystemClock),
        Partition::Staged,
    );
    let (meta, bytes) = &items[0];
    staged
        .import(meta, Box::new(Cursor::new(bytes.clone())))
        .await
        .unwrap();

    let open = admin.open_rewrite().await.unwrap().unwrap();
    assert_eq!(
        (open.kind, open.old_key, open.staged),
        (RewriteKind::Rotate, Some(old.key_id()), 1)
    );
    // Without the old key or its words, the items cannot be read.
    match admin.resume_rewrite(&words(NEW), None, None).await {
        Err(StoreError::Encryption(EncryptionError::NoKey)) => {}
        other => panic!("unexpected {other:?}"),
    }
    let finished = admin
        .resume_rewrite(&words(NEW), None, Some(&words(OLD)))
        .await
        .unwrap();
    assert_eq!(finished.key_id(), new.key_id());
    assert_holds(&device.store(Some(&new)), &items).await;
}
