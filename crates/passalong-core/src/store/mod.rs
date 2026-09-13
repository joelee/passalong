//! Storage: the backend-agnostic [`Store`] trait and its implementations.
//!
//! Commands depend only on `dyn Store`. [`FsStore`] implements it on top of
//! any [`RemoteFs`](crate::fs::RemoteFs), which covers the local and SSH
//! backends; a future web API or S3 backend can implement [`Store`]
//! directly.

pub mod factory;
pub mod fs_store;

pub use factory::{BackendFuture, BackendOpener, BackendRegistry, open_store};
pub use fs_store::FsStore;

use std::time::Duration;

use async_trait::async_trait;

use crate::fs::{BoxRead, FsError};
use crate::model::{ContentKey, ItemId, ItemMeta, ModelError, NewItem};

/// Result of [`Store::put`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PutOutcome {
    /// The stored item, or the existing item with the same content.
    pub meta: ItemMeta,
    /// `false` when identical content was already stored and nothing was
    /// uploaded.
    pub created: bool,
}

/// A place items are stored.
#[async_trait]
pub trait Store: Send + Sync {
    /// Stores `content` as a new item, streaming it without buffering the
    /// whole content. Identical content already in the store is not stored
    /// again: the existing item is returned with `created: false`.
    async fn put(&self, item: NewItem, content: BoxRead) -> Result<PutOutcome, StoreError>;

    /// Lists every item, newest first.
    async fn list(&self) -> Result<Vec<ItemMeta>, StoreError>;

    /// Items newer than `after`, newest first, or every item for `None`.
    /// Only those items' metadata is read, so polling for new items stays
    /// cheap on a large store.
    async fn list_after(&self, after: Option<&ItemId>) -> Result<Vec<ItemMeta>, StoreError>;

    /// Every item's id, newest first, without reading any metadata. The
    /// default lists every item; backends override it with something
    /// cheaper.
    async fn list_ids(&self) -> Result<Vec<ItemId>, StoreError> {
        Ok(self
            .list_after(None)
            .await?
            .into_iter()
            .map(|meta| meta.id)
            .collect())
    }

    /// The newest item's id, or `None` for an empty store. The default
    /// lists every item; backends override it with something cheaper.
    #[deprecated(
        since = "0.1.3",
        note = "pull mode no longer uses it; use list_ids and take the first"
    )]
    async fn newest_id(&self) -> Result<Option<ItemId>, StoreError> {
        Ok(self
            .list_after(None)
            .await?
            .into_iter()
            .next()
            .map(|meta| meta.id))
    }

    /// Returns an item's metadata and a stream of its content.
    async fn get(&self, id: &ItemId) -> Result<(ItemMeta, BoxRead), StoreError>;

    /// An item's metadata alone, without opening its content. The default
    /// calls [`Store::get`] and drops the content; backends override it
    /// with a single metadata read.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] for an unknown id, or the backend's error.
    async fn get_meta(&self, id: &ItemId) -> Result<ItemMeta, StoreError> {
        Ok(self.get(id).await?.0)
    }

    /// Whether an item exists.
    async fn exists(&self, id: &ItemId) -> Result<bool, StoreError>;

    /// Finds the oldest item whose content has `key`.
    async fn find_by_content_key(&self, key: &ContentKey) -> Result<Option<ItemMeta>, StoreError>;

    /// Turns what a user typed into one item id. Input without `-` is a
    /// prefix of the content key (for example `2cf2`); input with `-` is a
    /// prefix of the full id. Case and surrounding spaces are ignored, and
    /// at least 4 characters are required.
    async fn resolve(&self, input: &str) -> Result<ItemId, StoreError>;

    /// Deletes an item and returns its metadata. The item disappears from
    /// listings in one step, even if removing its data then fails.
    async fn delete(&self, id: &ItemId) -> Result<ItemMeta, StoreError>;

    /// Removes staging directories that interrupted uploads and deletions
    /// left behind, when older than `older_than`, and returns how many were
    /// removed. Younger ones may belong to uploads in progress and stay.
    async fn clean_staging(&self, older_than: Duration) -> Result<usize, StoreError>;
}

/// Storage errors.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// No item matches the given id or prefix.
    #[error("no item matches `{0}`")]
    NotFound(String),
    /// A prefix matches more than one item.
    #[error("`{input}` matches {} items: {}", candidates.len(), join_ids(candidates))]
    Ambiguous {
        /// What the user typed.
        input: String,
        /// Every matching id, newest first.
        candidates: Vec<ItemId>,
    },
    /// The input is too short to identify an item.
    #[error("`{0}` is too short: give at least 4 characters of the id")]
    InvalidPrefix(String),
    /// `server.kind` names a backend this build does not provide.
    #[error("unsupported storage backend `{0}`")]
    UnsupportedBackend(String),
    /// The backend's configuration is incomplete.
    #[error("storage configuration: {0}")]
    Config(String),
    /// A filesystem operation failed.
    #[error(transparent)]
    Fs(#[from] FsError),
    /// Reading the content being stored failed.
    #[error("reading the content failed: {0}")]
    Content(String),
    /// An item's stored data is unreadable or inconsistent.
    #[error("item {id} is corrupt: {reason}")]
    Corrupt {
        /// The item's id.
        id: String,
        /// What is wrong.
        reason: String,
    },
    /// Building item metadata failed.
    #[error(transparent)]
    Model(#[from] ModelError),
    /// A backend-specific failure, such as an SSH connection error.
    #[error("{0}")]
    Backend(String),
}

fn join_ids(ids: &[ItemId]) -> String {
    ids.iter()
        .map(ItemId::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}
