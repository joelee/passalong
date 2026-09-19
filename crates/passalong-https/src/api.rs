//! The server's JSON documents, as its `openapi.json` defines them. Fields
//! this client does not know are ignored, as the contract asks. Byte counts
//! travel as strings, since item sizes are 64-bit.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer};
use serde_json::value::RawValue;

/// `getViewer`: the API key in use, and the server.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Viewer {
    /// The key.
    pub key: ViewerKey,
    /// The server.
    pub server: ServerInfo,
}

/// The API key, as the server knows it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ViewerKey {
    /// Its public id.
    pub id: String,
    /// Its label, such as the device's name.
    #[serde(default)]
    pub label: Option<String>,
    /// What it may do.
    pub role: Role,
    /// When it expires, RFC 3339; `None` for never.
    #[serde(default)]
    pub expires_at: Option<String>,
}

/// What an API key may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum Role {
    /// Read and write.
    ReadWrite,
    /// Read only.
    ReadOnly,
    /// A role this client does not know.
    #[serde(other)]
    Unknown,
}

/// The server's version and limits.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ServerInfo {
    /// The server's version.
    pub version: String,
    /// The API version it speaks.
    pub api_version: u32,
    /// The largest item it takes; `None` for no limit.
    #[serde(default, deserialize_with = "optional_bytes")]
    pub max_item_bytes: Option<u64>,
    /// Where the server's source is.
    #[serde(default)]
    pub source_url: Option<String>,
}

/// `getWorkspace`: the workspace the API key opens.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Workspace {
    /// Its name.
    pub name: String,
    /// Its quota.
    #[serde(deserialize_with = "bytes")]
    pub quota_bytes: u64,
    /// The bytes its items take.
    #[serde(deserialize_with = "bytes")]
    pub used_bytes: u64,
    /// How many items it holds.
    pub item_count: u64,
    /// Its encryption.
    pub encryption: Encryption,
}

/// A workspace's encryption, as the server says it is.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Encryption {
    /// Plaintext, sealed, or being rewritten.
    pub state: EncryptionState,
    /// The key id readers need; while a rewrite is open, the one before it.
    #[serde(default)]
    pub key_id: Option<String>,
    /// The store header, as the client wrote it.
    #[serde(default)]
    pub header: Option<Box<RawValue>>,
    /// The rewrite session, when one is open.
    #[serde(default)]
    pub rewrite: Option<RewriteSession>,
}

/// A workspace's encryption state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum EncryptionState {
    /// Not encrypted.
    Plaintext,
    /// Encrypted.
    Sealed,
    /// A migration or rotation is open.
    Rewriting,
    /// A state this client does not know.
    #[serde(other)]
    Unknown,
}

/// An open migration or rotation.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct RewriteSession {
    /// `migrate` or `rotate`.
    pub kind: String,
    /// The public id of the API key that holds the lease.
    pub holder: String,
    /// When the lease ends, RFC 3339.
    pub lease_expires_at: String,
    /// The new key's id.
    pub new_key_id: String,
    /// Ids already staged under the new key.
    #[serde(default)]
    pub staged_ids: Vec<String>,
    /// How many items the workspace held when it began.
    pub source_items: u64,
    /// The new header, which the new words unlock (PLAN-00010 D-05).
    #[serde(default)]
    pub new_header: Option<Box<RawValue>>,
}

/// A byte count sent as a string of digits.
fn bytes<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    let text = String::deserialize(deserializer)?;
    text.parse()
        .map_err(|_| D::Error::custom(format!("`{text}` is not a byte count")))
}

fn optional_bytes<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<u64>, D::Error> {
    match Option::<String>::deserialize(deserializer)? {
        Some(text) => text
            .parse()
            .map(Some)
            .map_err(|_| D::Error::custom(format!("`{text}` is not a byte count"))),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_viewer_reads_as_the_server_sends_it() {
        let viewer: Viewer = serde_json::from_str(
            r#"{"key":{"expiresAt":null,"id":"052a09a4bd92","label":"curl","role":"readWrite"},"server":{"apiVersion":1,"maxItemBytes":null,"sourceUrl":"https://github.com/joelee/passalong-server","version":"0.1.0"}}"#,
        )
        .unwrap();
        assert_eq!(viewer.key.id, "052a09a4bd92");
        assert_eq!(viewer.key.role, Role::ReadWrite);
        assert_eq!(viewer.server.max_item_bytes, None);
        let limited: ServerInfo = serde_json::from_str(
            r#"{"version":"9","apiVersion":1,"maxItemBytes":"18446744073709551615","later":true}"#,
        )
        .unwrap();
        assert_eq!(limited.max_item_bytes, Some(u64::MAX));
        assert!(
            serde_json::from_str::<ServerInfo>(
                r#"{"version":"9","apiVersion":1,"maxItemBytes":"-1"}"#
            )
            .is_err()
        );
    }

    #[test]
    fn a_workspace_keeps_its_header_as_it_was_written() {
        let workspace: Workspace = serde_json::from_str(
            r#"{"name":"home","quotaBytes":"21474836480","usedBytes":"5","itemCount":1,"encryption":{"state":"sealed","keyId":"0011223344556677","header":{ "b": 1,  "a": [2] }}}"#,
        )
        .unwrap();
        assert_eq!(workspace.quota_bytes, 21_474_836_480);
        assert_eq!(workspace.encryption.state, EncryptionState::Sealed);
        assert_eq!(
            workspace.encryption.header.unwrap().get(),
            r#"{ "b": 1,  "a": [2] }"#
        );
        let other: Encryption = serde_json::from_str(r#"{"state":"somethingNew"}"#).unwrap();
        assert_eq!(other.state, EncryptionState::Unknown);
    }
}
