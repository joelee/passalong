//! [`HttpStore`]: a passalong-server workspace as a [`Store`].
//!
//! Items have the bytes `FsStore` gives them, through `store::format`: the
//! server keeps `meta` and the content as they are sent. In a sealed
//! workspace both are sealed here, and everything read is opened and checked
//! here; the server authenticates nothing about content.
//!
//! Every write names the key the client believes the workspace is under
//! (`expectedKeyId`), so the server refuses it atomically when that changed.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use passalong_core::clock::Clock;
use passalong_core::crypto::{self, KeyId, Sealer};
use passalong_core::encryption::EncryptionError;
use passalong_core::fs::BoxRead;
use passalong_core::model::{ContentDigest, ContentKey, ItemId, ItemMeta, NewItem};
use passalong_core::store::format::{self, CopyError, MetaError};
use passalong_core::store::{PutOutcome, Store, StoreError, WriteProbe};
use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE, RANGE};
use reqwest::{StatusCode, Url};
use serde_json::value::RawValue;
use tokio::sync::OnceCell;

use crate::api::{self, BeginUpload, Cleaned, Resolved, UploadTicket};
use crate::client::{Client, json};
use crate::error::{Code, HttpsError};

/// How many times a download that broke off is resumed.
pub const MAX_RESUMES: u32 = 3;

/// Which of a workspace's generations a store reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Partition {
    /// The workspace's items.
    Current,
    /// The items a fresh start set aside, unencrypted.
    Plain,
    /// An open rewrite's next generation; its holder alone reads it.
    Staged,
}

impl Partition {
    fn name(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Plain => "plain",
            Self::Staged => "staged",
        }
    }
}

/// A passalong-server workspace.
pub struct HttpStore {
    client: Arc<Client>,
    sealer: Option<Sealer>,
    clock: Arc<dyn Clock>,
    partition: Partition,
    /// The server's `maxItemBytes`, read before the first upload.
    limit: OnceCell<Option<u64>>,
    /// The workspace's key id, which the plain partition's writes name.
    workspace_key: OnceCell<Option<String>>,
}

impl std::fmt::Debug for HttpStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpStore")
            .field("client", &self.client)
            .field("sealed", &self.sealer.is_some())
            .field("partition", &self.partition)
            .finish_non_exhaustive()
    }
}

/// The store error for a failed request: the key and rewrite codes as the
/// encryption errors every backend gives, anything else in the server's
/// words.
pub fn store_error(err: HttpsError) -> StoreError {
    match err.code() {
        Some(Code::KeyIdMismatch) => EncryptionError::KeyChanged.into(),
        Some(Code::RewriteInProgress) => EncryptionError::Rewriting { started: None }.into(),
        _ => StoreError::Backend(err.to_string()),
    }
}

fn is_not_found(err: &HttpsError) -> bool {
    err.code() == Some(Code::NotFound)
}

fn corrupt(id: &str, reason: impl std::fmt::Display) -> StoreError {
    StoreError::Corrupt {
        id: id.to_owned(),
        reason: reason.to_string(),
    }
}

impl HttpStore {
    /// The workspace behind `client`: sealed with `sealer`, or plaintext.
    pub fn new(client: Arc<Client>, sealer: Option<Sealer>, clock: Arc<dyn Clock>) -> Self {
        Self::in_partition(client, sealer, clock, Partition::Current)
    }

    /// The workspace's `partition`.
    pub fn in_partition(
        client: Arc<Client>,
        sealer: Option<Sealer>,
        clock: Arc<dyn Clock>,
        partition: Partition,
    ) -> Self {
        Self {
            client,
            sealer,
            clock,
            partition,
            limit: OnceCell::new(),
            workspace_key: OnceCell::new(),
        }
    }

    /// The client this store sends with.
    pub fn client(&self) -> &Arc<Client> {
        &self.client
    }

    fn url(&self, path: &str, query: &[(&str, &str)]) -> Result<Url, StoreError> {
        let mut url = Url::parse(&self.client.url(path))
            .map_err(|err| StoreError::Config(format!("server.https.url: {err}")))?;
        let partition = (self.partition != Partition::Current).then(|| self.partition.name());
        if partition.is_some() || !query.is_empty() {
            let mut pairs = url.query_pairs_mut();
            if let Some(partition) = partition {
                pairs.append_pair("partition", partition);
            }
            for (name, value) in query {
                pairs.append_pair(name, value);
            }
        }
        Ok(url)
    }

    /// The key id every write names: the store's key, or for the plain
    /// partition, which holds plaintext items in a sealed workspace, the
    /// workspace's key.
    async fn expected(&self) -> Result<Option<String>, StoreError> {
        if self.partition != Partition::Plain {
            return Ok(self
                .sealer
                .as_ref()
                .map(|sealer| sealer.key_id().to_string()));
        }
        self.workspace_key
            .get_or_try_init(|| async {
                let workspace = self.client.workspace().await.map_err(store_error)?;
                Ok::<_, StoreError>(workspace.encryption.key_id)
            })
            .await
            .cloned()
    }

    /// Warns, as the filesystem store does, while items a fresh start set
    /// aside remain unencrypted.
    async fn remind_plain_left(&self) {
        if self.sealer.is_none() || self.partition != Partition::Current {
            return;
        }
        let Ok(mut url) = Url::parse(&self.client.url("/item-ids")) else {
            return;
        };
        url.query_pairs_mut().append_pair("partition", "plain");
        let Ok(response) = self
            .client
            .send("listItemIds", |http| http.get(url.clone()))
            .await
        else {
            return;
        };
        let n = json::<Vec<String>>(response)
            .await
            .map_or(0, |ids| ids.len());
        if n > 0 {
            let (items, them) = if n == 1 {
                ("item remains", "it")
            } else {
                ("items remain", "them")
            };
            tracing::warn!(
                "{n} unencrypted {items} from before encryption; remove {them} with `passalong prune --plain`"
            );
        }
    }

    /// An envelope's metadata and, when sealed, its content salt.
    fn decode(
        &self,
        item: &api::Item,
    ) -> Result<(ItemMeta, Option<format::ContentSalt>), StoreError> {
        let id = ItemId::parse(&item.id).map_err(|err| corrupt(&item.id, err))?;
        format::decode_meta(&id, item.meta.get().as_bytes(), self.sealer.as_ref()).map_err(|err| {
            match err {
                // A server's item is complete when it is listed, so one
                // that does not open is damaged, never still arriving.
                MetaError::Corrupt(reason) | MetaError::Unopened(reason) => {
                    corrupt(&item.id, reason)
                }
            }
        })
    }

    async fn item(&self, id: &ItemId) -> Result<Option<api::Item>, StoreError> {
        let url = self.url(&format!("/items/{id}"), &[])?;
        match self
            .client
            .send("getItem", |http| http.get(url.clone()))
            .await
        {
            Ok(response) => Ok(Some(json(response).await.map_err(store_error)?)),
            Err(err) if is_not_found(&err) => Ok(None),
            Err(err) => Err(store_error(err)),
        }
    }

    async fn limit(&self) -> Result<Option<u64>, StoreError> {
        self.limit
            .get_or_try_init(|| async {
                let viewer = self.client.viewer().await.map_err(store_error)?;
                Ok::<_, StoreError>(viewer.server.max_item_bytes)
            })
            .await
            .copied()
    }

    /// Settles an upload whose commit was lost: the id was ours, so the
    /// item is there or it is not.
    async fn settle(&self, id: &ItemId) -> Result<PutOutcome, StoreError> {
        match self.item(id).await? {
            Some(item) => Ok(PutOutcome {
                meta: self.decode(&item)?.0,
                created: true,
            }),
            None => Err(StoreError::Backend(format!(
                "the upload of {id} was lost; send it again"
            ))),
        }
    }

    /// Sends the stored bytes in `path` to the ticket, then commits it.
    async fn send_and_commit(
        &self,
        ticket: &UploadTicket,
        path: &std::path::Path,
        size: u64,
        id: &ItemId,
    ) -> Result<PutOutcome, StoreError> {
        let content = self.url(&format!("/uploads/{}/content", ticket.upload_id), &[])?;
        self.client
            .send("putUploadContent", |http| {
                http.put(content.clone())
                    .header(CONTENT_LENGTH, size)
                    .body(file_body(path))
            })
            .await
            .map_err(store_error)?;
        let commit = self.url(&format!("/uploads/{}/commit", ticket.upload_id), &[])?;
        match self
            .client
            .send("commitUpload", |http| http.post(commit.clone()))
            .await
        {
            Ok(response) => {
                let outcome: api::PutOutcome = json(response).await.map_err(store_error)?;
                Ok(PutOutcome {
                    meta: self.decode(&outcome.item)?.0,
                    created: outcome.created,
                })
            }
            Err(err) if is_not_found(&err) => self.settle(id).await,
            Err(err) => Err(store_error(err)),
        }
    }

    async fn abort(&self, ticket: &UploadTicket) {
        let Ok(url) = self.url(&format!("/uploads/{}", ticket.upload_id), &[]) else {
            return;
        };
        if let Err(err) = self
            .client
            .send("abortUpload", |http| http.delete(url.clone()))
            .await
        {
            tracing::debug!(error = %err, "could not abort the upload; the server's janitor will");
        }
    }
}

/// A request body that streams the file at `path`.
fn file_body(path: &std::path::Path) -> reqwest::Body {
    match std::fs::File::open(path) {
        Ok(file) => reqwest::Body::wrap_stream(tokio_util::io::ReaderStream::new(
            tokio::fs::File::from_std(file),
        )),
        Err(err) => reqwest::Body::wrap_stream(futures_util::stream::iter([Err::<Bytes, _>(err)])),
    }
}

fn copy_error(err: CopyError) -> StoreError {
    match err {
        CopyError::Read(err) => StoreError::Content(err.to_string()),
        CopyError::Write(err) => StoreError::Backend(format!("writing a temporary file: {err}")),
        CopyError::Crypto(err) => err.into(),
    }
}

fn temp_error(err: io::Error) -> StoreError {
    StoreError::Backend(format!("a temporary file: {err}"))
}

#[async_trait]
impl Store for HttpStore {
    fn key_id(&self) -> Option<KeyId> {
        self.sealer.as_ref().map(Sealer::key_id)
    }

    fn content_key(&self, digest: &ContentDigest) -> ContentKey {
        match &self.sealer {
            Some(sealer) => sealer.content_key(digest),
            None => digest.content_key(),
        }
    }

    async fn put(&self, item: NewItem, content: BoxRead) -> Result<PutOutcome, StoreError> {
        // The id and metadata come before the content, so the content is
        // hashed first, into a file only this user may read.
        let spool = tempfile::NamedTempFile::new().map_err(temp_error)?;
        let mut file = tokio::fs::File::from_std(spool.reopen().map_err(temp_error)?);
        let written = format::write_content(content, &mut file, None)
            .await
            .map_err(copy_error)?;
        let key = self.content_key(&written.digest);
        let preview = format::preview(item.kind, &written.head);
        let meta = item.finish_keyed(self.clock.now(), &written.digest, key, preview)?;

        let (stored, salt) = match &self.sealer {
            Some(sealer) => {
                let sealing = sealer.content_sealer()?;
                let salt = sealing.salt();
                let sealed = tempfile::NamedTempFile::new().map_err(temp_error)?;
                let mut out = tokio::fs::File::from_std(sealed.reopen().map_err(temp_error)?);
                let input = tokio::fs::File::open(spool.path())
                    .await
                    .map_err(temp_error)?;
                format::seal_content(input, &mut out, sealing)
                    .await
                    .map_err(copy_error)?;
                (sealed, Some(salt))
            }
            None => (spool, None),
        };
        let size = stored.as_file().metadata().map_err(temp_error)?.len();
        if let Some(limit) = self.limit().await?
            && size > limit
        {
            return Err(StoreError::Backend(format!(
                "the item takes {size} bytes on the server, over its limit of {limit} bytes for one item"
            )));
        }

        let sealing = self.sealer.as_ref().zip(salt.as_ref());
        let meta_json = format::encode_meta(&meta, sealing).map_err(|err| match err {
            format::EncodeError::Crypto(err) => err.into(),
            format::EncodeError::Json(err) => corrupt(meta.id.as_str(), err),
        })?;
        let text = String::from_utf8(meta_json).map_err(|err| corrupt(meta.id.as_str(), err))?;
        // A JSON value cannot carry the file's final newline.
        let raw = RawValue::from_string(text.trim_end().to_owned())
            .map_err(|err| corrupt(meta.id.as_str(), err))?;
        let request = serde_json::to_vec(&BeginUpload {
            id: meta.id.as_str(),
            meta: &raw,
            size: size.to_string(),
            expected_key_id: self.expected().await?,
            in_rewrite: false,
        })
        .map_err(|err| corrupt(meta.id.as_str(), err))?;
        let uploads = self.url("/uploads", &[])?;
        let response = self
            .client
            .send("beginUpload", |http| {
                http.post(uploads.clone())
                    .header(CONTENT_TYPE, "application/json")
                    .body(request.clone())
            })
            .await
            .map_err(store_error)?;
        if response.status() == StatusCode::OK {
            // The content is stored already: nothing is sent.
            let outcome: api::PutOutcome = json(response).await.map_err(store_error)?;
            tracing::info!(id = %outcome.item.id, "item already present");
            return Ok(PutOutcome {
                meta: self.decode(&outcome.item)?.0,
                created: false,
            });
        }
        let ticket: UploadTicket = json(response).await.map_err(store_error)?;
        match self
            .send_and_commit(&ticket, stored.path(), size, &meta.id)
            .await
        {
            Ok(outcome) => {
                tracing::info!(id = %outcome.meta.id, size = outcome.meta.size, created = outcome.created, "item stored");
                Ok(outcome)
            }
            Err(err) => {
                self.abort(&ticket).await;
                Err(err)
            }
        }
    }

    async fn list(&self) -> Result<Vec<ItemMeta>, StoreError> {
        self.remind_plain_left().await;
        self.list_after(None).await
    }

    async fn list_after(&self, after: Option<&ItemId>) -> Result<Vec<ItemMeta>, StoreError> {
        let url = match after {
            Some(after) => self.url("/items", &[("after", after.as_str())])?,
            None => self.url("/items", &[])?,
        };
        let response = self
            .client
            .send("listItems", |http| http.get(url.clone()))
            .await
            .map_err(store_error)?;
        let items: Vec<api::Item> = json(response).await.map_err(store_error)?;
        let mut metas = Vec::with_capacity(items.len());
        for item in &items {
            match self.decode(item) {
                Ok((meta, _)) => metas.push(meta),
                Err(err) => tracing::warn!(id = %item.id, error = %err, "skipping corrupt item"),
            }
        }
        Ok(metas)
    }

    async fn list_ids(&self) -> Result<Vec<ItemId>, StoreError> {
        let url = self.url("/item-ids", &[])?;
        let response = self
            .client
            .send("listItemIds", |http| http.get(url.clone()))
            .await
            .map_err(store_error)?;
        let ids: Vec<String> = json(response).await.map_err(store_error)?;
        Ok(ids.iter().filter_map(|id| ItemId::parse(id).ok()).collect())
    }

    async fn get(&self, id: &ItemId) -> Result<(ItemMeta, BoxRead), StoreError> {
        let item = self
            .item(id)
            .await?
            .ok_or_else(|| StoreError::NotFound(id.to_string()))?;
        let (meta, salt) = self.decode(&item)?;
        let url = self.url(&format!("/items/{id}/content"), &[])?;
        let content = download(self.client.clone(), url, item.stored_bytes);
        match (&self.sealer, salt) {
            (Some(sealer), Some(salt)) => {
                let expected = crypto::sealed_len(meta.size);
                if item.stored_bytes != expected {
                    return Err(corrupt(
                        id.as_str(),
                        format!("content has {} of {expected} bytes", item.stored_bytes),
                    ));
                }
                Ok((meta, Box::new(sealer.open_item_content(content, salt))))
            }
            _ => Ok((meta, content)),
        }
    }

    async fn get_meta(&self, id: &ItemId) -> Result<ItemMeta, StoreError> {
        match self.item(id).await? {
            Some(item) => Ok(self.decode(&item)?.0),
            None => Err(StoreError::NotFound(id.to_string())),
        }
    }

    async fn exists(&self, id: &ItemId) -> Result<bool, StoreError> {
        Ok(self.item(id).await?.is_some())
    }

    async fn find_by_content_key(&self, key: &ContentKey) -> Result<Option<ItemMeta>, StoreError> {
        let url = self.url(&format!("/content-keys/{key}"), &[])?;
        match self
            .client
            .send("findByContentKey", |http| http.get(url.clone()))
            .await
        {
            Ok(response) => {
                let item: api::Item = json(response).await.map_err(store_error)?;
                Ok(Some(self.decode(&item)?.0))
            }
            Err(err) if is_not_found(&err) => Ok(None),
            Err(err) => Err(store_error(err)),
        }
    }

    async fn resolve(&self, input: &str) -> Result<ItemId, StoreError> {
        let url = self.url("/items/resolve", &[("input", input)])?;
        let response = self
            .client
            .send("resolveItem", |http| http.get(url.clone()))
            .await
            .map_err(store_error)?;
        let resolved: Resolved = json(response).await.map_err(store_error)?;
        let parse = |id: &str| ItemId::parse(id).map_err(|err| corrupt(id, err));
        match resolved.result.as_str() {
            "resolved" => parse(resolved.id.as_deref().unwrap_or_default()),
            "ambiguous" => Err(StoreError::Ambiguous {
                input: input.trim().to_owned(),
                candidates: resolved
                    .candidates
                    .iter()
                    .map(|id| parse(id))
                    .collect::<Result<_, _>>()?,
            }),
            "invalidPrefix" => Err(StoreError::InvalidPrefix(input.trim().to_owned())),
            _ => Err(StoreError::NotFound(input.trim().to_owned())),
        }
    }

    async fn delete(&self, id: &ItemId) -> Result<ItemMeta, StoreError> {
        let expected = self.expected().await?;
        let mut query = Vec::new();
        if let Some(key) = &expected {
            query.push(("expectedKeyId", key.as_str()));
        }
        let url = self.url(&format!("/items/{id}"), &query)?;
        match self
            .client
            .send("deleteItem", |http| http.delete(url.clone()))
            .await
        {
            Ok(response) => {
                let item: api::Item = json(response).await.map_err(store_error)?;
                Ok(self.decode(&item)?.0)
            }
            Err(err) if is_not_found(&err) => Err(StoreError::NotFound(id.to_string())),
            Err(err) => Err(store_error(err)),
        }
    }

    async fn clean_staging(&self, older_than: Duration) -> Result<usize, StoreError> {
        let url = self.url("/workspace/clean-staging", &[])?;
        let body = serde_json::json!({ "olderThanSecs": older_than.as_secs() }).to_string();
        let response = self
            .client
            .send("cleanStaging", |http| {
                http.post(url.clone())
                    .header(CONTENT_TYPE, "application/json")
                    .body(body.clone())
            })
            .await
            .map_err(store_error)?;
        let cleaned: Cleaned = json(response).await.map_err(store_error)?;
        Ok(cleaned.removed)
    }

    async fn probe_write(&self) -> Result<WriteProbe, StoreError> {
        let url = self.url("/workspace/probe", &[])?;
        self.client
            .send("probeWrite", |http| http.post(url.clone()))
            .await
            .map_err(store_error)?;
        Ok(WriteProbe::Verified)
    }
}

/// An item's content, read from `url`. A read that breaks off before
/// `total` bytes is resumed with `Range` from the last byte received, at
/// most [`MAX_RESUMES`] times; the reader's caller checks the whole, by its
/// SHA-256 or its seal.
fn download(client: Arc<Client>, url: Url, total: u64) -> BoxRead {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(8);
    tokio::spawn(async move {
        let mut offset = 0_u64;
        let mut resumes = 0;
        loop {
            let range = (offset > 0).then(|| format!("bytes={offset}-"));
            let response = client
                .send("getItemContent", |http| {
                    let request = http.get(url.clone());
                    match &range {
                        Some(range) => request.header(RANGE, range),
                        None => request,
                    }
                })
                .await;
            let mut response = match response {
                Ok(response) => response,
                Err(err) => {
                    let _ = tx.send(Err(io::Error::other(err.to_string()))).await;
                    return;
                }
            };
            if offset > 0 && response.status() != StatusCode::PARTIAL_CONTENT {
                let _ = tx
                    .send(Err(io::Error::other(
                        "the server did not resume the download where it broke off",
                    )))
                    .await;
                return;
            }
            let broke = loop {
                match response.chunk().await {
                    Ok(Some(chunk)) => {
                        offset += chunk.len() as u64;
                        if tx.send(Ok(chunk)).await.is_err() {
                            return;
                        }
                    }
                    Ok(None) => break None,
                    Err(err) => break Some(err.to_string()),
                }
            };
            if broke.is_none() && offset >= total {
                return;
            }
            resumes += 1;
            if resumes > MAX_RESUMES {
                let reason = broke.unwrap_or_else(|| "it ended early".to_owned());
                let _ = tx
                    .send(Err(io::Error::other(format!(
                        "the download broke off at {offset} of {total} bytes: {reason}"
                    ))))
                    .await;
                return;
            }
            tracing::info!(offset, total, "download broke off; resuming");
        }
    });
    let stream = Box::pin(futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|item| (item, rx))
    }));
    Box::new(tokio_util::io::StreamReader::new(stream))
}
