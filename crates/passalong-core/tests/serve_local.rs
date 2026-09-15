//! `serve::run` end to end: a real drop folder, a local store, and a
//! scripted clipboard.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use passalong_core::clipboard::RgbaImage;
use passalong_core::clock::SystemClock;
use passalong_core::config::AfterSend;
use passalong_core::crypto::{DataKey, KdfParams, Words};
use passalong_core::encryption;
use passalong_core::fs::LocalFs;
use passalong_core::model::{ItemKind, ItemMeta};
use passalong_core::random::StdRandom;
use passalong_core::serve::{self, ServeError, ServeOptions, StoreOpener};
use passalong_core::store::{BackendFuture, FsStore, Store, StoreError};
use passalong_core::testing::MockClipboard;
use tempfile::TempDir;
use tokio::sync::{oneshot, watch};

fn local_opener(root: PathBuf) -> StoreOpener {
    Arc::new(move || -> BackendFuture<'static> {
        let root = root.clone();
        Box::pin(async move {
            tokio::fs::create_dir_all(&root).await.unwrap();
            let store: Box<dyn Store> = Box::new(FsStore::new(
                LocalFs::new(root),
                Arc::new(SystemClock),
                Box::new(StdRandom::new()),
            ));
            Ok(store)
        })
    })
}

fn options(drop: &Path) -> ServeOptions {
    ServeOptions {
        drop_folder: drop.to_path_buf(),
        clipboard_poll_interval: Duration::from_millis(20),
        file_stable_wait: Duration::from_millis(100),
        rescan_interval: Duration::from_millis(250),
        after_send: AfterSend::Move,
        device: "it".into(),
        clipboard_images: true,
        pull: false,
        pull_interval: Duration::from_millis(50),
        download_dir: drop.with_file_name("downloads"),
    }
}

fn test_image() -> RgbaImage {
    RgbaImage::new(2, 2, (0..16).map(|i| i * 13).collect()).unwrap()
}

async fn items(root: &Path) -> Vec<ItemMeta> {
    FsStore::new(
        LocalFs::new(root),
        Arc::new(SystemClock),
        Box::new(StdRandom::new()),
    )
    .list()
    .await
    .unwrap()
}

async fn wait_for(what: &str, mut done: impl AsyncFnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !done().await {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serve_sends_clipboard_text_and_dropped_files_then_stops() {
    let dir = TempDir::new().unwrap();
    let (store, drop) = (dir.path().join("store"), dir.path().join("drop"));
    std::fs::create_dir_all(&drop).unwrap();
    std::fs::write(drop.join("early.txt"), b"was here before serve").unwrap();
    let clipboard = MockClipboard::new().with_reads([None, Some("hello"), Some("hello")]);
    let (stop, stopped) = watch::channel(false);
    let task = tokio::spawn(serve::run(
        options(&drop),
        Some(Box::new(clipboard)),
        local_opener(store.clone()),
        stopped,
    ));

    wait_for("the sent folder", async || drop.join("sent").is_dir()).await;
    std::fs::write(drop.join("report.txt"), b"dropped while running").unwrap();
    wait_for("three items", async || items(&store).await.len() == 3).await;
    wait_for("report.txt to move", async || {
        drop.join("sent/report.txt").exists()
    })
    .await;
    assert!(!drop.join("report.txt").exists() && !drop.join("early.txt").exists());
    assert!(drop.join("sent/early.txt").exists());

    let listed = items(&store).await;
    assert_eq!(
        listed.iter().filter(|m| m.kind == ItemKind::Text).count(),
        1,
        "clipboard text sent once"
    );
    let mut names: Vec<_> = listed.iter().filter_map(|m| m.name.clone()).collect();
    names.sort();
    assert_eq!(names, ["early.txt", "report.txt"]);

    stop.send(true).unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("serve stops promptly");
    result.unwrap().unwrap();
}

#[tokio::test]
async fn serve_works_without_a_clipboard() {
    let dir = TempDir::new().unwrap();
    let (store, drop) = (dir.path().join("store"), dir.path().join("drop"));
    let (stop, stopped) = watch::channel(false);
    let task = tokio::spawn(serve::run(
        options(&drop),
        None,
        local_opener(store.clone()),
        stopped,
    ));
    wait_for("the sent folder", async || drop.join("sent").is_dir()).await;
    std::fs::write(drop.join("only.txt"), b"x").unwrap();
    wait_for("one item", async || items(&store).await.len() == 1).await;
    std::mem::drop(stop);
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn serve_fails_fast_when_the_store_cannot_be_opened() {
    let dir = TempDir::new().unwrap();
    let opener: StoreOpener = Arc::new(|| -> BackendFuture<'static> {
        Box::pin(async { Err(StoreError::Backend("server down".into())) })
    });
    let (_stop, stopped) = watch::channel(false);
    let err = serve::run(options(&dir.path().join("drop")), None, opener, stopped)
        .await
        .unwrap_err();
    assert!(matches!(err, ServeError::Store(_)), "{err:?}");
    assert!(err.to_string().contains("server down"), "{err}");
}

#[tokio::test]
async fn serve_fails_when_the_drop_folder_cannot_be_created() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("a-file");
    std::fs::write(&file, b"").unwrap();
    let (_stop, stopped) = watch::channel(false);
    let err = serve::run(
        options(&file.join("drop")),
        None,
        local_opener(dir.path().join("store")),
        stopped,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ServeError::DropFolder { .. }), "{err:?}");
}

#[tokio::test]
async fn serve_signals_readiness_after_start_up() {
    let dir = TempDir::new().unwrap();
    let (store, drop_folder) = (dir.path().join("store"), dir.path().join("drop"));
    let (stop, stopped) = watch::channel(false);
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(serve::run_with_ready(
        options(&drop_folder),
        None,
        local_opener(store),
        stopped,
        ready_tx,
    ));
    tokio::time::timeout(Duration::from_secs(5), ready_rx)
        .await
        .expect("ready in time")
        .expect("ready sent");
    assert!(
        drop_folder.join("sent").is_dir(),
        "ready only after the drop folder is prepared"
    );
    stop.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

/// A file `serve` cannot read is skipped, then sent once it is fixed.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serve_retries_a_skipped_file_after_it_changes() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new().unwrap();
    let (store, drop_folder) = (dir.path().join("store"), dir.path().join("drop"));
    std::fs::create_dir_all(&drop_folder).unwrap();
    let locked = drop_folder.join("locked.txt");
    std::fs::write(&locked, b"secret until fixed").unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&locked).is_ok() {
        return; // running as root: permissions are not enforced
    }
    let (stop, stopped) = watch::channel(false);
    let task = tokio::spawn(serve::run(
        options(&drop_folder),
        None,
        local_opener(store.clone()),
        stopped,
    ));
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    assert!(!drop_folder.join("sent/locked.txt").exists());
    assert!(items(&store).await.is_empty());

    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).unwrap();
    let later = std::time::SystemTime::now() + Duration::from_secs(2);
    std::fs::File::options()
        .write(true)
        .open(&locked)
        .unwrap()
        .set_modified(later)
        .unwrap();
    wait_for("the fixed file to be sent", async || {
        drop_folder.join("sent/locked.txt").exists()
    })
    .await;
    assert_eq!(items(&store).await.len(), 1);
    stop.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

async fn stop_and_join(
    stop: watch::Sender<bool>,
    task: tokio::task::JoinHandle<Result<(), ServeError>>,
) {
    stop.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("serve stops promptly")
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serve_sends_a_clipboard_image_once() {
    let dir = TempDir::new().unwrap();
    let (store, drop) = (dir.path().join("store"), dir.path().join("drop"));
    let clipboard = MockClipboard::with_image(test_image());
    let (stop, stopped) = watch::channel(false);
    let task = tokio::spawn(serve::run(
        options(&drop),
        Some(Box::new(clipboard)),
        local_opener(store.clone()),
        stopped,
    ));
    wait_for("the image", async || {
        items(&store).await.iter().any(ItemMeta::is_clipboard_image)
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(items(&store).await.len(), 1, "sent once while unchanged");
    stop_and_join(stop, task).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serve_prefers_text_over_an_image() {
    let dir = TempDir::new().unwrap();
    let (store, drop) = (dir.path().join("store"), dir.path().join("drop"));
    let clipboard = MockClipboard::with_text("words")
        .with_image_reads(std::iter::repeat_n(Some(test_image()), 100));
    let (stop, stopped) = watch::channel(false);
    let task = tokio::spawn(serve::run(
        options(&drop),
        Some(Box::new(clipboard)),
        local_opener(store.clone()),
        stopped,
    ));
    wait_for("the text", async || items(&store).await.len() == 1).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let listed = items(&store).await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].kind, ItemKind::Text);
    stop_and_join(stop, task).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serve_ignores_images_when_turned_off() {
    let dir = TempDir::new().unwrap();
    let (store, drop) = (dir.path().join("store"), dir.path().join("drop"));
    std::fs::create_dir_all(&drop).unwrap();
    std::fs::write(drop.join("proof.txt"), b"serve is running").unwrap();
    let mut opts = options(&drop);
    opts.clipboard_images = false;
    let (stop, stopped) = watch::channel(false);
    let task = tokio::spawn(serve::run(
        opts,
        Some(Box::new(MockClipboard::with_image(test_image()))),
        local_opener(store.clone()),
        stopped,
    ));
    wait_for("the dropped file", async || items(&store).await.len() == 1).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let listed = items(&store).await;
    assert_eq!(listed.len(), 1);
    assert!(!listed[0].is_clipboard_image());
    stop_and_join(stop, task).await;
}

/// Stores text as another device would, straight into the shared store.
async fn put_from_phone(root: &Path, text: &str) {
    FsStore::new(
        LocalFs::new(root),
        Arc::new(SystemClock),
        Box::new(StdRandom::new()),
    )
    .put(
        passalong_core::model::NewItem::text("phone"),
        Box::new(std::io::Cursor::new(text.as_bytes().to_vec())),
    )
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serve_pull_applies_text_from_other_devices_and_does_not_send_it_back() {
    let dir = TempDir::new().unwrap();
    let (store, drop) = (dir.path().join("store"), dir.path().join("drop"));
    let mut opts = options(&drop);
    opts.pull = true;
    let clipboard = MockClipboard::new();
    let (stop, stopped) = watch::channel(false);
    let (ready_tx, ready_rx) = oneshot::channel();
    let task = tokio::spawn(serve::run_with_ready(
        opts,
        Some(Box::new(clipboard.clone())),
        local_opener(store.clone()),
        stopped,
        ready_tx,
    ));
    ready_rx.await.unwrap();
    put_from_phone(&store, "from the phone").await;
    wait_for("the pulled text", async || {
        clipboard.current().as_deref() == Some("from the phone")
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let listed = items(&store).await;
    assert_eq!(listed.len(), 1, "pulled text is not sent back");
    assert_eq!(listed[0].device, "phone");
    stop_and_join(stop, task).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serve_without_pull_leaves_other_devices_items_alone() {
    let dir = TempDir::new().unwrap();
    let (store, drop) = (dir.path().join("store"), dir.path().join("drop"));
    let downloads = dir.path().join("downloads");
    std::fs::create_dir_all(&downloads).unwrap();
    let clipboard = MockClipboard::new();
    let (stop, stopped) = watch::channel(false);
    let (ready_tx, ready_rx) = oneshot::channel();
    let task = tokio::spawn(serve::run_with_ready(
        options(&drop),
        Some(Box::new(clipboard.clone())),
        local_opener(store.clone()),
        stopped,
        ready_tx,
    ));
    ready_rx.await.unwrap();
    put_from_phone(&store, "not for this device").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(clipboard.writes().is_empty());
    assert_eq!(std::fs::read_dir(&downloads).unwrap().count(), 0);
    stop_and_join(stop, task).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serve_sends_the_image_behind_a_copied_image_link() {
    let dir = TempDir::new().unwrap();
    let (store, drop) = (dir.path().join("store"), dir.path().join("drop"));
    let clipboard = MockClipboard::with_text("https://cdn.example.com/a.webp")
        .with_image_reads(std::iter::repeat_n(Some(test_image()), 100));
    let (stop, stopped) = watch::channel(false);
    let task = tokio::spawn(serve::run(
        options(&drop),
        Some(Box::new(clipboard)),
        local_opener(store.clone()),
        stopped,
    ));
    wait_for("the image", async || {
        items(&store).await.iter().any(ItemMeta::is_clipboard_image)
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let listed = items(&store).await;
    assert_eq!(listed.len(), 1, "the link is not sent as text");
    stop_and_join(stop, task).await;
}

fn sealed_opener(root: PathBuf, key: DataKey) -> StoreOpener {
    Arc::new(move || -> BackendFuture<'static> {
        let (root, key) = (root.clone(), key.clone());
        Box::pin(async move {
            encryption::open_with_key(
                LocalFs::new(root),
                Some(key),
                Arc::new(SystemClock),
                Box::new(StdRandom::new()),
            )
            .await
        })
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serve_sends_into_an_encrypted_store_leaving_no_plaintext() {
    let dir = TempDir::new().unwrap();
    let (store, drop) = (dir.path().join("store"), dir.path().join("drop"));
    std::fs::create_dir_all(&drop).unwrap();
    std::fs::create_dir_all(&store).unwrap();
    let kdf = KdfParams {
        m_kib: 64,
        t: 1,
        p: 1,
        salt: [1; 16],
    };
    let words = Words::parse("zoom zoom zoom zoom zoom zoom").unwrap();
    let key = encryption::set_up(&LocalFs::new(&store), &words, kdf)
        .await
        .unwrap();
    let clipboard = MockClipboard::new().with_reads([Some("secret clipboard text")]);
    let (stop, stopped) = watch::channel(false);
    let task = tokio::spawn(serve::run(
        options(&drop),
        Some(Box::new(clipboard)),
        sealed_opener(store.clone(), key.clone()),
        stopped,
    ));
    wait_for("the sent folder", async || drop.join("sent").is_dir()).await;
    std::fs::write(drop.join("note.txt"), b"a dropped secret").unwrap();
    let listed = async || {
        encryption::open_with_key(
            LocalFs::new(&store),
            Some(key.clone()),
            Arc::new(SystemClock),
            Box::new(StdRandom::new()),
        )
        .await
        .unwrap()
        .list()
        .await
        .unwrap()
    };
    wait_for("two sealed items", async || listed().await.len() == 2).await;
    let items = listed().await;
    assert!(items.iter().any(|m| m.kind == ItemKind::Text));
    assert!(items.iter().any(|m| m.name.as_deref() == Some("note.txt")));

    let mut pending = vec![store.clone()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let bytes = std::fs::read(&path).unwrap();
                for secret in ["secret clipboard text", "a dropped secret", "note.txt"] {
                    assert!(
                        !bytes.windows(secret.len()).any(|w| w == secret.as_bytes()),
                        "{} holds `{secret}`",
                        path.display()
                    );
                }
            }
        }
    }

    stop.send(true).unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("serve stops promptly");
    result.unwrap().unwrap();
}
