//! The client against a real passalong-server: connecting, TLS, and
//! refusals. Ignored by default; `just test-https` runs them.

mod support;

use std::time::{Duration, Instant};

use passalong_core::api_key::ApiKey;
use passalong_https::api::{EncryptionState, Role};
use passalong_https::{Client, Code, HttpsError};
use support::TestServer;
use tempfile::TempDir;

fn client(server: &TestServer, dir: &TempDir, pin: Option<&str>, key: ApiKey) -> Client {
    let config = server.config(dir.path(), pin);
    Client::new(config.server.https.as_ref().unwrap(), key).unwrap()
}

#[tokio::test]
#[ignore = "needs passalong-server: just test-https"]
async fn the_pinned_server_answers_in_either_pin_form() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let pin = server.pin.to_string();
    for form in [pin.clone(), pin.replacen("sha256/", "sha256//", 1)] {
        let viewer = client(&server, &dir, Some(&form), server.key.clone())
            .viewer()
            .await
            .unwrap();
        // The key's public id is read the way the server names it.
        assert_eq!(viewer.key.id, server.key.id());
        assert_eq!(viewer.key.role, Role::ReadWrite);
        assert_eq!(viewer.key.expires_at, None);
        assert_eq!(viewer.server.api_version, 1);
    }
    let workspace = client(&server, &dir, Some(&pin), server.key.clone())
        .workspace()
        .await
        .unwrap();
    assert_eq!(workspace.name, support::WORKSPACE);
    assert_eq!(workspace.item_count, 0);
    assert_eq!(workspace.encryption.state, EncryptionState::Plaintext);
}

#[tokio::test]
#[ignore = "needs passalong-server: just test-https"]
async fn a_wrong_pin_fails_before_any_request_naming_the_right_one() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let wrong = "sha256/Zmh6rfhivXdsj8GLjp+OIAiXFIVu4jOzkCpZHQ1fKSU=";
    let started = Instant::now();
    let err = client(&server, &dir, Some(wrong), server.key.clone())
        .viewer()
        .await
        .unwrap_err();
    match &err {
        HttpsError::Transport {
            reason, retryable, ..
        } => {
            assert!(reason.contains(&server.pin.to_string()), "{reason}");
            assert!(!retryable);
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(started.elapsed() < Duration::from_secs(1), "retried: {err}");
    // No request reached the server: the key was never used.
    let keys: serde_json::Value =
        serde_json::from_str(&server.run(&["key", "list", "--json"])).unwrap();
    let key = keys
        .as_array()
        .unwrap()
        .iter()
        .find(|key| key["id"] == server.key.id())
        .unwrap();
    assert!(key["lastUsedAt"].is_null(), "{key}");
}

#[tokio::test]
#[ignore = "needs passalong-server: just test-https"]
async fn without_a_pin_a_self_signed_server_is_not_trusted() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let err = client(&server, &dir, None, server.key.clone())
        .viewer()
        .await
        .unwrap_err();
    match err {
        HttpsError::Transport {
            reason, retryable, ..
        } => {
            assert!(reason.contains("certificate"), "{reason}");
            assert!(!retryable, "{reason}");
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
#[ignore = "needs passalong-server: just test-https"]
async fn an_unknown_key_is_refused_once_and_never_repeated() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let stranger = ApiKey::parse(&format!("pal_{}_{}", "0".repeat(12), "0".repeat(64))).unwrap();
    let pin = server.pin.to_string();
    let started = Instant::now();
    let err = client(&server, &dir, Some(&pin), stranger)
        .viewer()
        .await
        .unwrap_err();
    assert_eq!(err.code(), Some(Code::Unauthenticated), "{err}");
    // A repeat would have waited a second first.
    assert!(started.elapsed() < Duration::from_secs(1), "retried: {err}");
}

#[tokio::test]
#[ignore = "needs passalong-server: just test-https"]
async fn revoked_and_read_only_keys_are_told_apart() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let pin = server.pin.to_string();
    let reader = server.create_key(&["--read-only", "--expires", "7d"]);
    let viewer = client(&server, &dir, Some(&pin), reader.clone())
        .viewer()
        .await
        .unwrap();
    assert_eq!(viewer.key.role, Role::ReadOnly);
    assert!(viewer.key.expires_at.is_some());

    server.run(&["key", "revoke", reader.id()]);
    let err = client(&server, &dir, Some(&pin), reader)
        .viewer()
        .await
        .unwrap_err();
    assert_eq!(err.code(), Some(Code::KeyRevoked), "{err}");
}
