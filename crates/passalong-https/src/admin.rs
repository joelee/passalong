//! [`HttpEncryptionAdmin`]: changing a workspace's encryption through the
//! server's API.
//!
//! What the filesystem does with a journal and renames is one call here:
//! set-up is `enableEncryption`, a fresh start `freshStart`, and a change of
//! words `replaceHeader`, each atomic on the server. The data key is made
//! and wrapped on the device, as for every store; the server keeps only the
//! wrapped key.
//!
//! Migrating and rotating run the shared engine, [`run_rewrite`], over the
//! server's rewrite session: `beginRewrite` takes a lease, each copy is an
//! upload into the staged generation, every copy is read back from there,
//! and `commitRewrite` switches generation, header, and key at once. The
//! server is the journal, so any device with the new words can finish it.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use passalong_core::clock::Clock;
use passalong_core::crypto::{DataKey, KdfParams, KeyId, Sealer, Words, unwrap, wrap};
use passalong_core::encryption::{
    EncryptionAdmin, EncryptionError, HEARTBEAT_EVERY, OpenRewrite, Rewrite, RewriteKind,
    StoreHeader, StoreState, run_rewrite,
};
use passalong_core::fs::BoxRead;
use passalong_core::model::{ItemId, ItemMeta};
use passalong_core::store::{Store, StoreError};
use reqwest::header::CONTENT_TYPE;

use crate::api::{self, EncryptionState};
use crate::client::{Client, json};
use crate::error::{Code, HttpsError};
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

    /// Sends a rewrite request and reads the session it answers with.
    async fn session_call(
        &self,
        what: &str,
        path: &str,
        body: Option<String>,
    ) -> Result<api::RewriteSession, HttpsError> {
        session_call(&self.client, what, path, body).await
    }

    /// Re-encrypts every item under a new key wrapped with `words`: from
    /// plaintext, or from the key `old` when rotating. A session whose
    /// key an abort ended is begun again under another key.
    async fn rewrite(
        &self,
        kind: RewriteKind,
        old: Option<&DataKey>,
        words: &Words,
        kdf: KdfParams,
    ) -> Result<DataKey, StoreError> {
        for attempt in 0..2 {
            let (key, header) = new_key(words, kdf)?;
            // A header that does not open with the words would lock the
            // workspace for good at the commit.
            check_header(&header, words, &key)?;
            let body = format!(
                "{{\"kind\":\"{}\",\"expectedKeyId\":{},\"newKeyId\":\"{}\",\"newHeader\":{}}}",
                match kind {
                    RewriteKind::Migrate => "migrate",
                    _ => "rotate",
                },
                match old {
                    Some(old) => format!("\"{}\"", old.key_id()),
                    None => "null".to_owned(),
                },
                key.key_id(),
                header_json(&header)
            );
            match self
                .session_call("beginRewrite", "/rewrite", Some(body))
                .await
            {
                Ok(session) => {
                    self.drive(&session, old.cloned(), &key).await?;
                    return Ok(key);
                }
                Err(err) if err.code() == Some(Code::RewriteEnded) && attempt == 0 => {
                    tracing::info!(
                        "the server ended a rewrite under that key; beginning again under a new one"
                    );
                }
                Err(err) => return Err(store_error(err)),
            }
        }
        Err(StoreError::Backend(
            "the server ended the rewrite twice; nothing was changed".to_owned(),
        ))
    }

    /// Runs the shared engine over the open session, from items under
    /// `old`, or plaintext, to `key`.
    async fn drive(
        &self,
        session: &api::RewriteSession,
        old: Option<DataKey>,
        key: &DataKey,
    ) -> Result<(), StoreError> {
        let rewrite = HttpRewrite {
            admin: self,
            source: HttpStore::new(
                self.client.clone(),
                old.map(Sealer::new),
                self.clock.clone(),
            ),
            target: HttpStore::in_partition(
                self.client.clone(),
                Some(Sealer::new(key.clone())),
                self.clock.clone(),
                Partition::Staged,
            ),
            new_key: key.key_id(),
        };
        let lease = keep_lease(self.client.clone(), heartbeat_every(session, Utc::now()));
        let result = run_rewrite(&rewrite).await;
        lease.abort();
        let items = result?;
        tracing::info!(items, key = %key.key_id().short(), "items re-encrypted, verified, and committed");
        Ok(())
    }

    /// The open session, and the workspace around it.
    async fn open_session(&self) -> Result<(api::Workspace, api::RewriteSession), StoreError> {
        let mut workspace = self.workspace().await?;
        match workspace.encryption.rewrite.take() {
            Some(session) => Ok((workspace, session)),
            None => Err(StoreError::Backend(
                "no re-encryption is open on the server".to_owned(),
            )),
        }
    }

    /// Makes this device the session's holder: renews its own lease, or
    /// takes over one that has ended.
    async fn hold(&self, session: &api::RewriteSession) -> Result<api::RewriteSession, StoreError> {
        let (what, path) = if session.holder == self.client.key().id() {
            ("heartbeatRewrite", "/rewrite/heartbeat")
        } else {
            ("takeOverRewrite", "/rewrite/take-over")
        };
        self.session_call(what, path, None)
            .await
            .map_err(store_error)
    }
}

/// Sends a rewrite request with `client` and reads the session it answers
/// with.
async fn session_call(
    client: &Client,
    what: &str,
    path: &str,
    body: Option<String>,
) -> Result<api::RewriteSession, HttpsError> {
    let url = client.url(path);
    let response = client
        .send(what, |http| {
            let request = http.post(&url);
            match &body {
                Some(body) => request
                    .header(CONTENT_TYPE, "application/json")
                    .body(body.clone()),
                None => request,
            }
        })
        .await?;
    json(response).await
}

/// Renews the lease every `every` until aborted: one item can take longer
/// than a lease on a slow link, so the renewal cannot wait between items.
fn keep_lease(client: Arc<Client>, every: Duration) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(every).await;
            match session_call(&client, "heartbeatRewrite", "/rewrite/heartbeat", None).await {
                Ok(_) => {}
                // Taken over: the next request of the rewrite says so.
                Err(err) if err.code() == Some(Code::LeaseHeld) => return,
                Err(err) => tracing::debug!(error = %err, "renewing the rewrite's lease failed"),
            }
        }
    })
}

/// How often a session's heartbeat is due: a third of its lease, and at
/// least once a minute.
fn heartbeat_every(session: &api::RewriteSession, now: DateTime<Utc>) -> Duration {
    let lease = DateTime::parse_from_rfc3339(&session.lease_expires_at)
        .ok()
        .and_then(|ends| (ends.with_timezone(&Utc) - now).to_std().ok())
        .unwrap_or(HEARTBEAT_EVERY);
    (lease / 3).clamp(Duration::from_millis(200), HEARTBEAT_EVERY)
}

/// Refuses a header that does not open with `words` to `key`.
fn check_header(header: &StoreHeader, words: &Words, key: &DataKey) -> Result<(), StoreError> {
    let opened = unwrap(header.wrapped(), words)?;
    if opened.key_id() != key.key_id() {
        return Err(EncryptionError::Header(format!(
            "the new header opens key {}, not {}; nothing was changed",
            opened.key_id().short(),
            key.key_id().short()
        ))
        .into());
    }
    Ok(())
}

fn rewrite_kind(session: &api::RewriteSession) -> Result<RewriteKind, StoreError> {
    match session.kind.as_str() {
        "migrate" => Ok(RewriteKind::Migrate),
        "rotate" => Ok(RewriteKind::Rotate),
        other => Err(StoreError::Backend(format!(
            "the server reports a rewrite of kind `{other}`, which this version of passalong does not know"
        ))),
    }
}

/// A re-encryption through the server's rewrite session.
struct HttpRewrite<'a> {
    admin: &'a HttpEncryptionAdmin,
    source: HttpStore,
    target: HttpStore,
    new_key: KeyId,
}

#[async_trait]
impl Rewrite for HttpRewrite<'_> {
    fn source(&self) -> &dyn Store {
        &self.source
    }

    fn target_id(&self, meta: &ItemMeta) -> Result<ItemId, StoreError> {
        self.target.id_for(meta)
    }

    async fn staged(&self, id: &ItemId) -> Result<bool, StoreError> {
        self.target.exists(id).await
    }

    async fn import(&self, meta: &ItemMeta, content: BoxRead) -> Result<ItemMeta, StoreError> {
        self.target.import(meta, content).await
    }

    async fn read_back(&self, id: &ItemId) -> Result<(ItemMeta, BoxRead), StoreError> {
        self.target.get(id).await
    }

    async fn heartbeat(&self) -> Result<(), StoreError> {
        // `keep_lease` renews it all along.
        Ok(())
    }

    async fn commit(&self) -> Result<(), StoreError> {
        self.admin
            .send_json(
                "commitRewrite",
                reqwest::Method::POST,
                "/rewrite/commit",
                format!("{{\"newKeyId\":\"{}\"}}", self.new_key),
            )
            .await
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
        words: &Words,
        kdf: KdfParams,
        _clock: Arc<dyn Clock>,
    ) -> Result<DataKey, StoreError> {
        match self.inspect().await? {
            StoreState::Plain { .. } => {}
            other => return Err(refuse(other)),
        }
        self.rewrite(RewriteKind::Migrate, None, words, kdf).await
    }

    async fn rotate(
        &self,
        old: &DataKey,
        words: &Words,
        kdf: KdfParams,
        _clock: Arc<dyn Clock>,
    ) -> Result<DataKey, StoreError> {
        match self.inspect().await? {
            StoreState::Encrypted { key_id, .. } if key_id == old.key_id() => {}
            StoreState::Encrypted { key_id, .. } => {
                return Err(EncryptionError::KeyMismatch {
                    device: old.key_id().short(),
                    store: key_id.short(),
                }
                .into());
            }
            StoreState::Plain { .. } => return Err(EncryptionError::NotEncrypted.into()),
            other => return Err(refuse(other)),
        }
        self.rewrite(RewriteKind::Rotate, Some(old), words, kdf)
            .await
    }

    async fn open_rewrite(&self) -> Result<Option<OpenRewrite>, StoreError> {
        let workspace = self.workspace().await?;
        let Some(session) = &workspace.encryption.rewrite else {
            return Ok(None);
        };
        let lease_ends = DateTime::parse_from_rfc3339(&session.lease_expires_at)
            .map_err(|err| {
                StoreError::Backend(format!(
                    "the server's leaseExpiresAt `{}` is not a time: {err}",
                    session.lease_expires_at
                ))
            })?
            .with_timezone(&Utc);
        let new_key = KeyId::parse(&session.new_key_id).map_err(|_| {
            StoreError::Backend(format!(
                "the server names the key id `{}`, which is not one",
                session.new_key_id
            ))
        })?;
        let kind = rewrite_kind(session)?;
        let old_key = match kind {
            RewriteKind::Migrate => None,
            _ => Some(key_id_of(&workspace.encryption)?),
        };
        Ok(Some(OpenRewrite {
            kind,
            mine: session.holder == self.client.key().id(),
            holder: session.holder.clone(),
            lease_ends,
            new_key,
            old_key,
            staged: session.staged_ids.len(),
            items: usize::try_from(session.source_items).unwrap_or(usize::MAX),
        }))
    }

    async fn resume_rewrite(
        &self,
        new: &Words,
        device: Option<&DataKey>,
        old_words: Option<&Words>,
    ) -> Result<DataKey, StoreError> {
        let (workspace, session) = self.open_session().await?;
        let raw = session.new_header.as_ref().ok_or_else(|| {
            StoreError::Backend(
                "the server does not give the rewrite's new header; it needs a passalong-server whose rewrite sessions carry newHeader".to_owned(),
            )
        })?;
        let header = StoreHeader::parse(raw.get().as_bytes())?;
        let key = unwrap(header.wrapped(), new)?;
        if key.key_id().to_string() != session.new_key_id {
            return Err(EncryptionError::Header(format!(
                "the rewrite's header opens key {}, but the server names key {}",
                key.key_id().short(),
                session.new_key_id
            ))
            .into());
        }
        let old = match rewrite_kind(&session)? {
            RewriteKind::Migrate => None,
            _ => {
                let old_id = key_id_of(&workspace.encryption)?;
                match (device, old_words) {
                    (Some(device), _) if device.key_id() == old_id => Some(device.clone()),
                    (_, Some(words)) => {
                        let raw = workspace
                            .encryption
                            .header
                            .as_ref()
                            .ok_or(EncryptionError::HeaderMissing)?;
                        let old =
                            unwrap(StoreHeader::parse(raw.get().as_bytes())?.wrapped(), words)?;
                        if old.key_id() != old_id {
                            return Err(EncryptionError::Header(format!(
                                "the old words open key {}, but the items are under key {}",
                                old.key_id().short(),
                                old_id.short()
                            ))
                            .into());
                        }
                        Some(old)
                    }
                    _ => return Err(EncryptionError::NoKey.into()),
                }
            }
        };
        let session = self.hold(&session).await?;
        self.drive(&session, old, &key).await?;
        Ok(key)
    }

    async fn abort_rewrite(&self) -> Result<(), StoreError> {
        let (_, session) = self.open_session().await?;
        let session = self.hold(&session).await?;
        self.send_json(
            "abortRewrite",
            reqwest::Method::POST,
            "/rewrite/abort",
            format!("{{\"newKeyId\":\"{}\"}}", session.new_key_id),
        )
        .await
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

#[cfg(test)]
mod tests {
    use super::*;

    fn quick() -> KdfParams {
        KdfParams {
            m_kib: 64,
            t: 1,
            p: 1,
            salt: [7; 16],
        }
    }

    #[test]
    fn a_header_that_opens_another_key_or_not_at_all_is_refused() {
        let words = Words::parse("abacus abdomen abdominal abide abiding ability").unwrap();
        let other = Words::parse("zoom zoom zoom zoom zoom zoom").unwrap();
        let (key, header) = new_key(&words, quick()).unwrap();
        check_header(&header, &words, &key).unwrap();
        assert!(check_header(&header, &other, &key).is_err());
        let (_, stranger) = new_key(&words, quick()).unwrap();
        match check_header(&stranger, &words, &key) {
            Err(StoreError::Encryption(EncryptionError::Header(reason))) => {
                assert!(reason.contains("nothing was changed"), "{reason}");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn the_heartbeat_comes_three_times_a_lease_and_at_least_once_a_minute() {
        let now = DateTime::parse_from_rfc3339("2026-09-19T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let session = |ends: &str| {
            serde_json::from_value::<api::RewriteSession>(serde_json::json!({
                "kind": "migrate", "holder": "k", "leaseExpiresAt": ends,
                "newKeyId": "x", "stagedIds": [], "sourceItems": 0,
            }))
            .unwrap()
        };
        assert_eq!(
            heartbeat_every(&session("2026-09-19T12:10:00Z"), now),
            HEARTBEAT_EVERY
        );
        assert_eq!(
            heartbeat_every(&session("2026-09-19T12:00:03Z"), now),
            Duration::from_secs(1)
        );
        assert_eq!(
            heartbeat_every(&session("2026-09-19T11:59:00Z"), now),
            HEARTBEAT_EVERY / 3,
            "an ended lease is renewed well within a minute"
        );
        assert_eq!(
            heartbeat_every(&session("2026-09-19T12:00:00.300Z"), now),
            Duration::from_millis(200)
        );
    }
}
