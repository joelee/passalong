//! The client against a deployed passalong-server (PLAN-00011). Ignored by
//! default; `just test-deployed` loads `.env` and runs them.

#[path = "support/deployed.rs"]
mod deployed;

use deployed::Deployed;
use passalong_https::Client;
use passalong_https::api::{EncryptionState, Role};
use tempfile::TempDir;

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
