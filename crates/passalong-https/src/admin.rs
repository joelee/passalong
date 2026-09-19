//! [`HttpEncryptionAdmin`]: changing a workspace's encryption through the
//! server's API.
//!
//! What the filesystem does with a journal and renames is one call here:
//! set-up is `enableEncryption`, a fresh start `freshStart`, and a change of
//! words `replaceHeader`, each atomic on the server. The data key is made
//! and wrapped on the device, as for every store; the server keeps only the
//! wrapped key.

use std::sync::Arc;

use async_trait::async_trait;
use passalong_core::clock::Clock;
use passalong_core::crypto::{DataKey, KdfParams, KeyId, Words, unwrap, wrap};
use passalong_core::encryption::{EncryptionAdmin, EncryptionError, StoreHeader, StoreState};
use passalong_core::store::{Store, StoreError};
use reqwest::header::CONTENT_TYPE;

use crate::api::{self, EncryptionState};
use crate::client::Client;
use crate::store::{HttpStore, Partition, store_error};

/// [`EncryptionAdmin`] for a passalong-server workspace.
pub struct HttpEncryptionAdmin {
    client: Arc<Client>,
    clock: Arc<dyn Clock>,
}

impl std::fmt::Debug for HttpEncryptionAdmin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpEncryptionAdmin")
            .field("client", &self.client)
            .finish_non_exhaustive()
    }
}

/// The refusal for a workspace in a state the operation cannot start from,
/// as the filesystem gives it.
fn refuse(state: StoreState) -> StoreError {
    match state {
        StoreState::Plain { .. } => EncryptionError::NotEncrypted.into(),
        StoreState::Encrypted { .. } => EncryptionError::AlreadyEncrypted.into(),
        StoreState::Rewriting { started } => EncryptionError::Rewriting { started }.into(),
        _ => EncryptionError::HeaderMissing.into(),
    }
}

fn new_key(words: &Words, kdf: KdfParams) -> Result<(DataKey, StoreHeader), StoreError> {
    let key = DataKey::generate()?;
    let header = StoreHeader::new(wrap(&key, words, kdf)?);
    Ok((key, header))
}

/// The header as JSON for a request body, without the file's last newline.
fn header_json(header: &StoreHeader) -> String {
    String::from_utf8_lossy(&header.to_json())
        .trim_end()
        .to_owned()
}

impl HttpEncryptionAdmin {
    /// Changes the encryption of the workspace behind `client`.
    pub fn new(client: Arc<Client>, clock: Arc<dyn Clock>) -> Self {
        Self { client, clock }
    }

    async fn workspace(&self) -> Result<api::Workspace, StoreError> {
        self.client.workspace().await.map_err(store_error)
    }

    fn state_of(
        &self,
        workspace: &api::Workspace,
        plain_left: usize,
    ) -> Result<StoreState, StoreError> {
        Ok(match workspace.encryption.state {
            EncryptionState::Plaintext => StoreState::Plain {
                items: usize::try_from(workspace.item_count).unwrap_or(usize::MAX),
            },
            EncryptionState::Sealed => StoreState::Encrypted {
                key_id: key_id_of(&workspace.encryption)?,
                plain_left,
            },
            EncryptionState::Rewriting => StoreState::Rewriting { started: None },
            _ => {
                return Err(StoreError::Backend(
                    "the server reports an encryption state this version of passalong does not know"
                        .to_owned(),
                ));
            }
        })
    }

    async fn send_json(
        &self,
        what: &str,
        method: reqwest::Method,
        path: &str,
        body: String,
    ) -> Result<(), StoreError> {
        let url = self.client.url(path);
        self.client
            .send(what, |http| {
                http.request(method.clone(), &url)
                    .header(CONTENT_TYPE, "application/json")
                    .body(body.clone())
            })
            .await
            .map_err(store_error)?;
        Ok(())
    }

    fn not_yet() -> StoreError {
        StoreError::Backend(
            "re-encrypting a passalong-server workspace is not supported by this build yet"
                .to_owned(),
        )
    }
}

/// The workspace's key id, which a sealed workspace must name.
fn key_id_of(encryption: &api::Encryption) -> Result<KeyId, StoreError> {
    let id = encryption.key_id.as_deref().unwrap_or_default();
    KeyId::parse(id).map_err(|_| {
        StoreError::Backend(format!(
            "the server names the key id `{id}`, which is not one"
        ))
    })
}

#[async_trait]
impl EncryptionAdmin for HttpEncryptionAdmin {
    async fn inspect(&self) -> Result<StoreState, StoreError> {
        let workspace = self.workspace().await?;
        let plain_left = if workspace.encryption.state == EncryptionState::Sealed {
            self.plain_store().list_ids().await?.len()
        } else {
            0
        };
        self.state_of(&workspace, plain_left)
    }

    async fn set_up(&self, words: &Words, kdf: KdfParams) -> Result<DataKey, StoreError> {
        match self.inspect().await? {
            StoreState::Plain { items: 0 } => {}
            StoreState::Plain { items } => {
                return Err(EncryptionError::Layout(format!(
                    "the store holds {items} items; encrypt it with a fresh start or by migrating"
                ))
                .into());
            }
            other => return Err(refuse(other)),
        }
        let (key, header) = new_key(words, kdf)?;
        let body = format!(
            "{{\"header\":{},\"keyId\":\"{}\"}}",
            header_json(&header),
            key.key_id()
        );
        self.send_json(
            "enableEncryption",
            reqwest::Method::PUT,
            "/workspace/encryption",
            body,
        )
        .await?;
        Ok(key)
    }

    async fn fresh_start(&self, words: &Words, kdf: KdfParams) -> Result<DataKey, StoreError> {
        match self.inspect().await? {
            StoreState::Plain { .. } => {}
            other => return Err(refuse(other)),
        }
        let (key, header) = new_key(words, kdf)?;
        let body = format!(
            "{{\"header\":{},\"keyId\":\"{}\"}}",
            header_json(&header),
            key.key_id()
        );
        self.send_json(
            "freshStart",
            reqwest::Method::POST,
            "/workspace/encryption/fresh-start",
            body,
        )
        .await?;
        Ok(key)
    }

    async fn join(&self, words: &Words) -> Result<DataKey, StoreError> {
        let workspace = self.workspace().await?;
        let state = self.state_of(&workspace, 0)?;
        let StoreState::Encrypted { key_id, .. } = state else {
            return Err(refuse(state));
        };
        let raw = workspace
            .encryption
            .header
            .as_ref()
            .ok_or(EncryptionError::HeaderMissing)?;
        let header = StoreHeader::parse(raw.get().as_bytes())?;
        if header.key_id() != key_id {
            return Err(EncryptionError::Header(format!(
                "the header is for key {}, but the server names key {}",
                header.key_id().short(),
                key_id.short()
            ))
            .into());
        }
        Ok(unwrap(header.wrapped(), words)?)
    }

    async fn change_words(
        &self,
        current: &Words,
        new: &Words,
        kdf: KdfParams,
    ) -> Result<KeyId, StoreError> {
        let key = self.join(current).await?;
        let header = StoreHeader::new(wrap(&key, new, kdf)?);
        let body = format!(
            "{{\"expectedKeyId\":\"{}\",\"header\":{}}}",
            key.key_id(),
            header_json(&header)
        );
        self.send_json(
            "replaceHeader",
            reqwest::Method::PUT,
            "/workspace/encryption/header",
            body,
        )
        .await?;
        Ok(key.key_id())
    }

    async fn migrate(
        &self,
        _words: &Words,
        _kdf: KdfParams,
        _clock: Arc<dyn Clock>,
    ) -> Result<DataKey, StoreError> {
        Err(Self::not_yet())
    }

    async fn rotate(
        &self,
        _old: &DataKey,
        _words: &Words,
        _kdf: KdfParams,
        _clock: Arc<dyn Clock>,
    ) -> Result<DataKey, StoreError> {
        Err(Self::not_yet())
    }

    fn plain_store(&self) -> Box<dyn Store + '_> {
        Box::new(HttpStore::in_partition(
            self.client.clone(),
            None,
            self.clock.clone(),
            Partition::Plain,
        ))
    }

    async fn remove_plain_if_empty(&self) -> Result<bool, StoreError> {
        // An empty plain partition costs nothing; the server's janitor
        // drops it.
        Ok(false)
    }
}
