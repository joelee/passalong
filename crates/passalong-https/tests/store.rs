//! `HttpStore` against a real passalong-server. Ignored by default;
//! `just test-https` runs them.

mod support;

use std::io::Cursor;

use passalong_core::api_key::{ApiKey, save_api_key};
use passalong_core::config::Config;
use passalong_core::crypto::DataKey;
use passalong_core::encryption::{EncryptionError, SystemGit, save_key_file};
use passalong_core::fs::BoxRead;
use passalong_core::model::{ContentHasher, ItemId, NewItem};
use passalong_core::store::format;
use passalong_core::store::{Store, StoreError, WriteProbe};
use passalong_https::{HttpStore, api, open_https_store};
use support::{CutProxy, Settings, TestServer};
use tempfile::TempDir;
use tokio::io::AsyncReadExt;

fn content(bytes: &[u8]) -> BoxRead {
    Box::new(Cursor::new(bytes.to_vec()))
}

async fn read_all(mut reader: BoxRead) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).await?;
    Ok(bytes)
}

/// Writes `key` where `config` keeps the API key.
fn give_key(config: &Config, key: &ApiKey) {
    let path = &config.server.https.as_ref().unwrap().api_key_file;
    save_api_key(path, key, &SystemGit::new()).unwrap();
}

async fn open(config: &Config, key: &ApiKey) -> HttpStore {
    give_key(config, key);
    open_https_store(config).await.unwrap()
}

fn text_key(text: &[u8]) -> passalong_core::model::ContentKey {
    let mut hasher = ContentHasher::new();
    hasher.update(text);
    hasher.finalize().content_key()
}

#[tokio::test]
#[ignore = "needs passalong-server: just test-https"]
async fn a_plaintext_workspace_stores_lists_and_serves_items() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let store = open(&server.pinned(dir.path()), &server.key).await;
    assert_eq!(store.key_id(), None);

    let text = store
        .put(NewItem::text("box"), content(b"hello"))
        .await
        .unwrap();
    assert!(text.created);
    let again = store
        .put(NewItem::text("laptop"), content(b"hello"))
        .await
        .unwrap();
    assert!(!again.created, "identical content is stored once");
    assert_eq!(again.meta.id, text.meta.id);
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let file = store
        .put(NewItem::file("report.pdf", "box"), content(b"%PDF-1.7 x"))
        .await
        .unwrap()
        .meta;

    let listed: Vec<ItemId> = store
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(
        listed,
        [file.id.clone(), text.meta.id.clone()],
        "newest first"
    );
    assert_eq!(store.list_ids().await.unwrap(), listed);
    let newer: Vec<ItemId> = store
        .list_after(Some(&text.meta.id))
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(newer, std::slice::from_ref(&file.id));

    let (meta, body) = store.get(&text.meta.id).await.unwrap();
    assert_eq!(meta, text.meta);
    assert_eq!(read_all(body).await.unwrap(), b"hello");
    assert_eq!(store.get_meta(&file.id).await.unwrap(), file);
    assert!(store.exists(&file.id).await.unwrap());
    let stranger = ItemId::parse("6aa52107-000000000000").unwrap();
    assert!(!store.exists(&stranger).await.unwrap());
    assert!(matches!(
        store.get(&stranger).await,
        Err(StoreError::NotFound(_))
    ));
    assert_eq!(
        store
            .find_by_content_key(&text_key(b"hello"))
            .await
            .unwrap()
            .map(|m| m.id),
        Some(text.meta.id.clone())
    );
    assert_eq!(
        store
            .find_by_content_key(&text_key(b"nobody"))
            .await
            .unwrap(),
        None
    );

    let prefix = &text.meta.id.as_str()[9..13];
    assert_eq!(store.resolve(prefix).await.unwrap(), text.meta.id);
    assert!(matches!(
        store.resolve("ab").await,
        Err(StoreError::InvalidPrefix(_))
    ));
    assert!(matches!(
        store.resolve("ffff").await,
        Err(StoreError::NotFound(_))
    ));

    // The server keeps `meta.json` as it was sent: all but the file's last
    // newline, which a JSON value cannot hold.
    let envelope: api::Item = store
        .client()
        .get_json(&format!("/items/{}", text.meta.id))
        .await
        .unwrap();
    let written = String::from_utf8(format::encode_meta(&text.meta, None).unwrap()).unwrap();
    assert_eq!(envelope.meta.get(), written.trim_end());

    assert_eq!(store.delete(&file.id).await.unwrap(), file);
    assert!(matches!(
        store.delete(&file.id).await,
        Err(StoreError::NotFound(_))
    ));
    assert_eq!(
        store
            .clean_staging(std::time::Duration::ZERO)
            .await
            .unwrap(),
        0
    );
    assert_eq!(store.probe_write().await.unwrap(), WriteProbe::Verified);
}

#[tokio::test]
#[ignore = "needs passalong-server: just test-https"]
async fn a_sealed_workspace_holds_nothing_readable_and_damage_is_found() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let config = server.pinned(dir.path());
    give_key(&config, &server.key);
    let client = passalong_https::connect(&config).unwrap();
    let key = server.seal(&client).await;
    save_key_file(
        config.client.key_file.as_ref().unwrap(),
        &key,
        &SystemGit::new(),
    )
    .unwrap();
    let store = open_https_store(&config).await.unwrap();
    assert_eq!(store.key_id(), Some(key.key_id()));

    let secret = b"the sealed text, never on the server";
    let stored = store
        .put(NewItem::text("box"), content(secret))
        .await
        .unwrap()
        .meta;
    let (meta, body) = store.get(&stored.id).await.unwrap();
    assert_eq!(meta, stored);
    assert_eq!(read_all(body).await.unwrap(), secret);

    let envelope: api::Item = client
        .get_json(&format!("/items/{}", stored.id))
        .await
        .unwrap();
    assert!(envelope.meta.get().contains("\"schema\": 2"));
    assert!(!envelope.meta.get().contains("sealed text"));
    let file = server.content_file(stored.id.as_str());
    let mut bytes = std::fs::read(&file).unwrap();
    assert!(!bytes.windows(10).any(|w| w == b"sealed tex"));

    // One altered byte on the server's disk, and the item does not open.
    *bytes.last_mut().unwrap() ^= 1;
    std::fs::write(&file, bytes).unwrap();
    let (_, body) = store.get(&stored.id).await.unwrap();
    assert!(read_all(body).await.is_err());
}

#[tokio::test]
#[ignore = "needs passalong-server: just test-https"]
async fn a_device_with_a_key_refuses_a_plaintext_workspace_before_writing() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let config = server.pinned(dir.path());
    give_key(&config, &server.key);
    save_key_file(
        config.client.key_file.as_ref().unwrap(),
        &DataKey::generate().unwrap(),
        &SystemGit::new(),
    )
    .unwrap();
    match open_https_store(&config).await {
        Err(StoreError::Encryption(EncryptionError::KeyWithoutEncryption)) => {}
        other => panic!("unexpected {other:?}"),
    }
    let workspace = passalong_https::connect(&config)
        .unwrap()
        .workspace()
        .await
        .unwrap();
    assert_eq!(workspace.item_count, 0);
}

#[tokio::test]
#[ignore = "needs passalong-server: just test-https"]
async fn an_item_over_the_server_s_limit_is_refused_before_any_upload() {
    let server = TestServer::start_with(Settings {
        max_item_bytes: Some(10),
        ..Settings::default()
    });
    let dir = TempDir::new().unwrap();
    let store = open(&server.pinned(dir.path()), &server.key).await;
    store
        .put(NewItem::text("box"), content(b"ten bytes!"))
        .await
        .unwrap();
    let err = store
        .put(NewItem::text("box"), content(b"eleven byte"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("limit of 10 bytes"), "{err}");
    let workspace = store.client().workspace().await.unwrap();
    assert_eq!((workspace.item_count, workspace.used_bytes), (1, 10));
}

#[tokio::test]
#[ignore = "needs passalong-server: just test-https"]
async fn a_download_cut_part_way_resumes_where_it_broke_off() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let data: Vec<u8> = (0..1024 * 1024).map(|i| (i * 7 % 251) as u8).collect();
    let id = open(&server.pinned(dir.path()), &server.key)
        .await
        .put(NewItem::file("big.bin", "box"), content(&data))
        .await
        .unwrap()
        .meta
        .id;

    let proxy = CutProxy::start(server.port(), 300 * 1024).await;
    let through = TempDir::new().unwrap();
    let store = open(&server.pinned_via(through.path(), proxy.port), &server.key).await;
    let (_, body) = store.get(&id).await.unwrap();
    assert_eq!(read_all(body).await.unwrap(), data);
}

#[tokio::test]
#[ignore = "needs passalong-server: just test-https"]
async fn a_read_only_key_reads_but_cannot_write() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    open(&server.pinned(dir.path()), &server.key)
        .await
        .put(NewItem::text("box"), content(b"shared"))
        .await
        .unwrap();
    let reader_dir = TempDir::new().unwrap();
    let reader = open(
        &server.pinned(reader_dir.path()),
        &server.create_key(&["--read-only"]),
    )
    .await;
    assert_eq!(reader.list().await.unwrap().len(), 1);
    let err = reader.probe_write().await.unwrap_err();
    assert!(err.to_string().contains("FORBIDDEN_ROLE"), "{err}");
    let err = reader
        .put(NewItem::text("box"), content(b"no"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("FORBIDDEN_ROLE"), "{err}");
}

#[tokio::test]
#[ignore = "needs passalong-server: just test-https"]
async fn every_write_names_the_key_it_expects() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let config = server.pinned(dir.path());
    let store = open(&config, &server.key).await;
    let kept = store
        .put(NewItem::text("box"), content(b"kept"))
        .await
        .unwrap()
        .meta;
    store.delete(&kept.id).await.unwrap();

    // Sealed behind its back, the workspace refuses this store's writes:
    // they name no key, and the server compares.
    let client = passalong_https::connect(&config).unwrap();
    server.seal(&client).await;
    match store.put(NewItem::text("box"), content(b"late")).await {
        Err(StoreError::Encryption(EncryptionError::KeyChanged)) => {}
        other => panic!("unexpected {other:?}"),
    }
    match store.delete(&kept.id).await {
        Err(StoreError::Encryption(EncryptionError::KeyChanged) | StoreError::NotFound(_)) => {}
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(client.workspace().await.unwrap().item_count, 0);
}

#[tokio::test]
#[ignore = "needs passalong-server: just test-https"]
async fn each_lookup_is_one_request_to_its_own_route() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let store = open(&server.pinned(dir.path()), &server.key).await;
    let first = store
        .put(NewItem::text("box"), content(b"one"))
        .await
        .unwrap()
        .meta;
    store
        .put(NewItem::text("box"), content(b"two"))
        .await
        .unwrap();

    // Waits for the server's log to show the requests since `before`.
    async fn since(server: &TestServer, before: usize) -> Vec<String> {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        server.operations()[before..].to_vec()
    }
    let prefix = &first.id.as_str()[9..13];
    let key = text_key(b"one");
    let checks: Vec<(&str, _)> = vec![
        (
            "listItems",
            Box::pin(async { store.list_after(Some(&first.id)).await.map(drop) })
                as std::pin::Pin<
                    Box<dyn std::future::Future<Output = Result<(), StoreError>> + '_>,
                >,
        ),
        (
            "listItemIds",
            Box::pin(async { store.list_ids().await.map(drop) }),
        ),
        (
            "getItem",
            Box::pin(async { store.get_meta(&first.id).await.map(drop) }),
        ),
        (
            "getItem",
            Box::pin(async { store.exists(&first.id).await.map(drop) }),
        ),
        (
            "findByContentKey",
            Box::pin(async { store.find_by_content_key(&key).await.map(drop) }),
        ),
        (
            "resolveItem",
            Box::pin(async { store.resolve(prefix).await.map(drop) }),
        ),
        (
            "cleanStaging",
            Box::pin(async {
                store
                    .clean_staging(std::time::Duration::ZERO)
                    .await
                    .map(drop)
            }),
        ),
        (
            "probeWrite",
            Box::pin(async { store.probe_write().await.map(drop) }),
        ),
    ];
    for (operation, call) in checks {
        let before = server.operations().len();
        call.await.unwrap();
        assert_eq!(since(&server, before).await, [operation]);
    }
}

#[tokio::test]
#[ignore = "needs passalong-server: just test-https"]
async fn a_full_workspace_names_its_quota() {
    let server = TestServer::start_with(Settings {
        quota_bytes: Some(20),
        ..Settings::default()
    });
    let dir = TempDir::new().unwrap();
    let store = open(&server.pinned(dir.path()), &server.key).await;
    assert_eq!(store.client().workspace().await.unwrap().quota_bytes, 20);
    store
        .put(NewItem::text("box"), content(b"fifteen bytes!!"))
        .await
        .unwrap();
    let err = store
        .put(NewItem::text("box"), content(b"ten bytes!"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("QUOTA_EXCEEDED"), "{err}");
    assert!(
        err.to_string().contains("of its quota of 20 bytes"),
        "{err}"
    );
}

#[tokio::test]
#[ignore = "needs passalong-server: just test-https"]
async fn debug_output_never_shows_the_api_key() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let config = server.pinned(dir.path());
    let store = open(&config, &server.key).await;
    let secret = server.key.expose().rsplit('_').next().unwrap().to_owned();
    for shown in [
        format!("{config:?}"),
        format!("{store:?}"),
        format!("{:?}", server.key),
    ] {
        assert!(!shown.contains(&secret), "{shown}");
    }
    assert!(format!("{store:?}").contains(server.key.id()));
}
