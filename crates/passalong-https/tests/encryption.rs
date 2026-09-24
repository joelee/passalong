//! `HttpEncryptionAdmin` against a real passalong-server: setting up,
//! joining, changing words, and a fresh start. Ignored by default;
//! `just test-https` runs them.

mod support;

use std::io::Cursor;
use std::sync::Arc;

use passalong_core::api_key::save_api_key;
use passalong_core::clock::SystemClock;
use passalong_core::config::Config;
use passalong_core::crypto::{KdfParams, Words};
use passalong_core::encryption::{EncryptionAdmin, EncryptionError, StoreState, SystemGit};
use passalong_core::model::NewItem;
use passalong_core::store::{Store, StoreError};
use passalong_https::{Client, Code, HttpEncryptionAdmin, HttpStore, HttpsError};
use support::TestServer;
use tempfile::TempDir;

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
        salt: [7; 16],
    }
}

fn connect(server: &TestServer, dir: &TempDir) -> (Config, Arc<Client>) {
    let config = server.pinned(dir.path());
    let path = &config.server.https.as_ref().unwrap().api_key_file;
    save_api_key(path, &server.key, &SystemGit::new()).unwrap();
    let client = passalong_https::connect(&config).unwrap();
    (config, client)
}

fn admin(client: &Arc<Client>) -> HttpEncryptionAdmin {
    HttpEncryptionAdmin::new(client.clone(), Arc::new(SystemClock))
}

#[tokio::test]
#[ignore = "needs passalong-server: just test-https"]
async fn set_up_join_and_a_change_of_words() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let (_, client) = connect(&server, &dir);
    let first = admin(&client);
    assert_eq!(
        first.inspect().await.unwrap(),
        StoreState::Plain { items: 0 }
    );

    let key = first.set_up(&words(W1), quick()).await.unwrap();
    assert_eq!(
        first.inspect().await.unwrap(),
        StoreState::Encrypted {
            key_id: key.key_id(),
            plain_left: 0
        }
    );
    match first.set_up(&words(W1), quick()).await {
        Err(StoreError::Encryption(EncryptionError::AlreadyEncrypted)) => {}
        other => panic!("unexpected {other:?}"),
    }

    // A second device, with its own configuration, joins with the words.
    let other = TempDir::new().unwrap();
    let (_, second_client) = connect(&server, &other);
    let second = admin(&second_client);
    let joined = second.join(&words(W1)).await.unwrap();
    assert_eq!(joined.key_id(), key.key_id());
    assert!(second.join(&words(W2)).await.is_err());

    let key_id = second
        .change_words(&words(W1), &words(W2), quick())
        .await
        .unwrap();
    assert_eq!(key_id, key.key_id(), "new words keep the data key");
    assert!(first.join(&words(W1)).await.is_err(), "the old words fail");
    assert_eq!(first.join(&words(W2)).await.unwrap().key_id(), key.key_id());
}

#[tokio::test]
#[ignore = "needs passalong-server: just test-https"]
async fn set_up_refuses_a_workspace_with_items() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let (_, client) = connect(&server, &dir);
    HttpStore::new(client.clone(), None, Arc::new(SystemClock))
        .put(NewItem::text("box"), Box::new(Cursor::new(b"old".to_vec())))
        .await
        .unwrap();
    let admin = admin(&client);
    match admin.set_up(&words(W1), quick()).await {
        Err(StoreError::Encryption(EncryptionError::Layout(reason))) => {
            assert!(reason.contains("fresh start"), "{reason}");
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(
        admin.inspect().await.unwrap(),
        StoreState::Plain { items: 1 }
    );
}

#[tokio::test]
#[ignore = "needs passalong-server: just test-https"]
async fn a_fresh_start_sets_old_items_aside_until_pruned() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let (_, client) = connect(&server, &dir);
    let plain = HttpStore::new(client.clone(), None, Arc::new(SystemClock));
    let old = plain
        .put(NewItem::text("box"), Box::new(Cursor::new(b"old".to_vec())))
        .await
        .unwrap()
        .meta;

    let admin = admin(&client);
    let key = admin.fresh_start(&words(W1), quick()).await.unwrap();
    assert_eq!(
        admin.inspect().await.unwrap(),
        StoreState::Encrypted {
            key_id: key.key_id(),
            plain_left: 1
        }
    );
    let workspace = client.workspace().await.unwrap();
    assert_eq!(workspace.item_count, 0, "the sealed partition starts empty");

    // The plain partition lists and deletes its items; its writes name the
    // workspace's key, which the server checks.
    let aside = admin.plain_store();
    assert_eq!(
        aside.list_ids().await.unwrap(),
        std::slice::from_ref(&old.id)
    );
    assert_eq!(aside.delete(&old.id).await.unwrap().id, old.id);
    assert!(aside.list_ids().await.unwrap().is_empty());
    assert!(!admin.remove_plain_if_empty().await.unwrap());
    assert_eq!(
        admin.inspect().await.unwrap(),
        StoreState::Encrypted {
            key_id: key.key_id(),
            plain_left: 0
        }
    );
}

#[tokio::test]
#[ignore = "needs passalong-server: just test-https"]
async fn enabling_encryption_again_is_a_replay_only_with_the_same_key() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let (_, client) = connect(&server, &dir);
    let key = server.seal(&client).await;
    let workspace = client.workspace().await.unwrap();
    let header = workspace.encryption.header.unwrap();
    let url = client.url("/workspace/encryption");
    let enable = |key_id: String| {
        let body = format!("{{\"header\":{},\"keyId\":\"{key_id}\"}}", header.get());
        let url = url.clone();
        let client = client.clone();
        async move {
            client
                .send("enableEncryption", |http| {
                    http.put(&url)
                        .header("content-type", "application/json")
                        .body(body.clone())
                })
                .await
        }
    };
    enable(key.key_id().to_string()).await.unwrap();
    let other = passalong_core::crypto::DataKey::generate().unwrap();
    match enable(other.key_id().to_string()).await {
        Err(err @ HttpsError::Refused { .. }) => {
            assert_eq!(err.code(), Some(Code::KeyIdMismatch), "{err}");
        }
        Err(err) => panic!("unexpected {err:?}"),
        Ok(_) => panic!("a second key was accepted"),
    }
}
