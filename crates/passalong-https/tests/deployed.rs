//! The client against a deployed passalong-server (PLAN-00011). Ignored by
//! default; `just test-deployed` loads `.env` and runs them.

#[path = "support/deployed.rs"]
mod deployed;

use std::time::{Duration, Instant};

use deployed::Deployed;
use passalong_core::api_key::ApiKey;
use passalong_core::config::TlsPin;
use passalong_https::api::{EncryptionState, Role};
use passalong_https::{Client, Code, HttpsError};
use tempfile::TempDir;

/// A client of `url` under `pin` with `key`, keeping its files in `dir`.
fn client(server: &Deployed, dir: &TempDir, url: &str, pin: Option<&str>, key: ApiKey) -> Client {
    let config = server.config(dir.path(), url, pin);
    Client::new(config.server.https.as_ref().unwrap(), key).unwrap()
}

/// The pin as `passalong-server tls fingerprint` prints it, `sha256/` and
/// the base64, and in curl's `sha256//` spelling. Base64 can itself begin
/// with `/`, as this deployment's does, so the forms come from the parsed
/// pin and not from counting slashes.
fn pin_forms(pin: &str) -> [String; 2] {
    let canonical = TlsPin::parse(pin).unwrap().to_string();
    let curl = canonical.replacen("sha256/", "sha256//", 1);
    [canonical, curl]
}

#[tokio::test]
#[ignore = "needs the deployed server: just test-deployed"]
async fn deployed_the_server_answers_the_viewer_and_workspace() {
    let server = Deployed::from_env();
    let dir = TempDir::new().unwrap();
    let config = server.pinned(dir.path());
    let client = Client::new(config.server.https.as_ref().unwrap(), server.key.clone()).unwrap();

    let viewer = client.viewer().await.unwrap();
    assert_eq!(viewer.key.id, server.key.id());
    assert_eq!(viewer.key.role, Role::ReadWrite);
    assert_eq!(viewer.key.expires_at, None, "the key never expires");
    assert_eq!(viewer.server.api_version, 1);

    let workspace = client.workspace().await.unwrap();
    assert_eq!(workspace.name, server.workspace);
    assert_eq!(workspace.quota_bytes, 256 * 1024 * 1024);
    assert_eq!(workspace.encryption.state, EncryptionState::Plaintext);
}

#[tokio::test]
#[ignore = "needs the deployed server: just test-deployed"]
async fn deployed_the_pin_works_in_either_form() {
    let server = Deployed::from_env();
    let dir = TempDir::new().unwrap();
    for form in pin_forms(&server.pin) {
        let viewer = client(&server, &dir, &server.url, Some(&form), server.key.clone())
            .viewer()
            .await
            .unwrap_or_else(|err| panic!("with pin {form}: {err}"));
        assert_eq!(viewer.key.id, server.key.id());
    }
}

#[tokio::test]
#[ignore = "needs the deployed server: just test-deployed"]
async fn deployed_a_wrong_pin_fails_before_any_request_naming_the_right_one() {
    let server = Deployed::from_env();
    let dir = TempDir::new().unwrap();
    let wrong = "sha256/Zmh6rfhivXdsj8GLjp+OIAiXFIVu4jOzkCpZHQ1fKSU=";
    let [right, _] = pin_forms(&server.pin);
    let started = Instant::now();
    let err = client(&server, &dir, &server.url, Some(wrong), server.key.clone())
        .viewer()
        .await
        .unwrap_err();
    let elapsed = started.elapsed();
    match &err {
        HttpsError::Transport {
            reason, retryable, ..
        } => {
            // The presented pin is named.
            assert!(reason.contains(&right), "{reason}");
            assert!(!retryable, "{reason}");
        }
        other => panic!("unexpected {other:?}"),
    }
    // A retry would have waited a second first.
    assert!(
        elapsed < Duration::from_secs(1),
        "retried ({elapsed:?}): {err}"
    );
}

#[tokio::test]
#[ignore = "needs the deployed server: just test-deployed"]
async fn deployed_every_name_on_the_certificate_presents_the_same_pin() {
    let server = Deployed::from_env();
    let dir = TempDir::new().unwrap();
    let mut urls = vec![server.url.clone()];
    urls.extend(server.alt_urls.iter().cloned());
    let mut failed = Vec::new();
    for url in &urls {
        // The pin alone is trusted, so a URL that answers under it carries
        // the same key; one that does not is named at the end.
        match client(&server, &dir, url, Some(&server.pin), server.key.clone())
            .viewer()
            .await
        {
            Ok(viewer) => assert_eq!(viewer.key.id, server.key.id(), "{url}"),
            Err(err) => failed.push(format!("{url}: {}", deployed::redact(&err.to_string()))),
        }
        let presented = passalong_https::tls::presented(url).await;
        match presented {
            Ok(presented) => {
                assert_eq!(presented.pin, TlsPin::parse(&server.pin).unwrap(), "{url}");
            }
            Err(err) => failed.push(format!("{url} (handshake): {err}")),
        }
    }
    assert!(
        failed.is_empty(),
        "unreachable under the pin:\n{}",
        failed.join("\n")
    );
}

#[tokio::test]
#[ignore = "needs the deployed server: just test-deployed"]
async fn deployed_an_unknown_key_is_refused_once() {
    let server = Deployed::from_env();
    let dir = TempDir::new().unwrap();
    let stranger = ApiKey::parse(&format!("pal_{}_{}", "0".repeat(12), "0".repeat(64))).unwrap();
    let started = Instant::now();
    let err = client(&server, &dir, &server.url, Some(&server.pin), stranger)
        .viewer()
        .await
        .unwrap_err();
    assert_eq!(err.code(), Some(Code::Unauthenticated), "{err}");
    // A repeat would have waited a second first.
    assert!(started.elapsed() < Duration::from_secs(1), "retried: {err}");
}

#[tokio::test]
#[ignore = "needs the deployed server: just test-deployed"]
async fn deployed_the_read_only_key_sees_but_cannot_write() {
    let server = Deployed::from_env();
    let dir = TempDir::new().unwrap();
    let reader = client(
        &server,
        &dir,
        &server.url,
        Some(&server.pin),
        server.read_only_key.clone(),
    );
    let viewer = reader.viewer().await.unwrap();
    assert_eq!(viewer.key.id, server.read_only_key.id());
    assert_eq!(viewer.key.role, Role::ReadOnly);
    assert_eq!(viewer.key.expires_at, None);
    let before = reader.workspace().await.unwrap();

    // The write probe is the smallest write there is; `check` skips it for
    // a read-only key, so it is sent here by hand.
    let url = reader.url("/workspace/probe");
    let err = reader
        .send("probeWrite", |http| http.post(&url))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Some(Code::ForbiddenRole), "{err}");

    let after = reader.workspace().await.unwrap();
    assert_eq!(after.item_count, before.item_count);
    assert_eq!(after.used_bytes, before.used_bytes);
}
