//! The journal of a change to a store's encryption, `.rewrite/plan.json`.
//! `.rewrite/` is also the lock: one change runs at a time.
//!
//! The journal is written whole into a staging folder, `.rewrite-<token>/`,
//! which is then renamed to `.rewrite/`. That rename is the lock: it fails
//! when `.rewrite/` exists, and because the staged folder is never empty it
//! fails too, rather than replacing anything, when another client's lock
//! appears in between. `.rewrite/` therefore never exists without a whole
//! journal, except one left by passalong 0.2.0, which created the folder
//! first and wrote its plan into it afterwards.
//!
//! A recovery claims `.rewrite/recovery/`, created exclusively, so two
//! recoveries never run at once.

use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::header::write_header_in;
use super::{EncryptionError, REWRITE_DIR, RewritePlan, StoreHeader};
use crate::crypto::random_bytes;
use crate::fs::{FsError, RemoteFs, RemotePath};
use crate::store::StoreError;

/// The journal's file name inside `.rewrite/`.
pub(super) const PLAN_FILE: &str = "plan.json";
/// The folder inside `.rewrite/` holding the header to put in place.
pub(super) const NEW_HEADER: &str = "header";
/// The folder inside `.rewrite/` keeping a copy of the header replaced.
pub(super) const OLD_HEADER: &str = "old-header";
/// The folder inside `.rewrite/` the replaced header moves into.
pub(super) const PREVIOUS: &str = "previous";
/// The folder a recovery claims inside `.rewrite/`.
const RECOVERY: &str = "recovery";
/// How staged journal folders start.
const STAGING_PREFIX: &str = ".rewrite-";
/// The longest a journal may be.
const MAX_JOURNAL_BYTES: u64 = 64 * 1024;

/// How long, in seconds, a recovery may run before another may take it
/// over: after that it has most likely stopped.
pub const RECOVERY_STALE_SECS: i64 = 10 * 60;

/// What the lock's journal says is being changed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Journal {
    /// Every item is being re-encrypted.
    Rewrite(RewritePlan),
    /// The store header is being set up or replaced.
    HeaderChange(HeaderChange),
    /// A change this version of passalong does not know.
    Newer {
        /// The journal's `kind`.
        kind: String,
    },
    /// A journal that cannot be read, such as one passalong 0.2.0 left half
    /// written.
    Unreadable {
        /// Why it cannot be read.
        reason: String,
    },
}

/// A change to the store header alone: setting up encryption, a fresh
/// start, or new words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct HeaderChange {
    /// Which change.
    pub kind: HeaderChangeKind,
    /// When it started, RFC 3339.
    pub started_at: String,
    /// Id of the key the new header wraps.
    pub key: String,
}

impl HeaderChange {
    /// The journal of a `kind` change to a header wrapping key `key`.
    pub fn new(kind: HeaderChangeKind, started_at: String, key: String) -> Self {
        Self {
            kind,
            started_at,
            key,
        }
    }
}

/// Which header change a [`HeaderChange`] records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum HeaderChangeKind {
    /// Encryption set up on an empty store.
    SetUp,
    /// Encryption set up with the earlier items moved to `plain/`.
    FreshStart,
    /// New words for the same key.
    Words,
}

/// `.rewrite/<rel>`.
pub(super) fn under_rewrite(rel: &str) -> Result<RemotePath, FsError> {
    RemotePath::new(&format!("{REWRITE_DIR}/{rel}"))
}

fn parse(bytes: &[u8]) -> Journal {
    let value: serde_json::Value = match serde_json::from_slice(bytes) {
        Ok(value) => value,
        Err(err) => {
            return Journal::Unreadable {
                reason: err.to_string(),
            };
        }
    };
    let kind = value
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let parsed = match kind.as_str() {
        "migrate" | "rotate" => serde_json::from_value(value).map(Journal::Rewrite),
        "set-up" | "fresh-start" | "words" => {
            serde_json::from_value(value).map(Journal::HeaderChange)
        }
        "" => {
            return Journal::Unreadable {
                reason: "it has no `kind`".to_owned(),
            };
        }
        _ => return Journal::Newer { kind },
    };
    parsed.unwrap_or_else(|err| Journal::Unreadable {
        reason: err.to_string(),
    })
}

/// Reads the lock's journal: `None` when the store is not locked, or its
/// lock has no journal.
///
/// # Errors
///
/// The filesystem's error. A journal that is there but cannot be parsed is
/// [`Journal::Unreadable`], not an error.
pub async fn read_journal<F: RemoteFs + ?Sized>(fs: &F) -> Result<Option<Journal>, StoreError> {
    let path = under_rewrite(PLAN_FILE)?;
    let reader = match fs.open_read(&path).await {
        Ok(reader) => reader,
        Err(FsError::NotFound(_)) => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    let mut bytes = Vec::new();
    reader
        .take(MAX_JOURNAL_BYTES)
        .read_to_end(&mut bytes)
        .await
        .map_err(|err| FsError::from_io(&path, err))?;
    Ok(Some(parse(&bytes)))
}

/// The refusal of an operation that needs a re-encryption's journal but
/// found `journal`.
pub(super) fn unsupported(journal: &Journal) -> StoreError {
    let reason = match journal {
        Journal::Rewrite(_) => "the store's items are being re-encrypted".to_owned(),
        Journal::HeaderChange(_) => {
            "the store's header is being changed, not its items re-encrypted".to_owned()
        }
        Journal::Newer { kind } => {
            format!("the lock's journal (`{kind}`) needs a newer version of passalong")
        }
        Journal::Unreadable { reason } => format!("the lock's journal cannot be read: {reason}"),
    };
    EncryptionError::Layout(reason).into()
}

/// The error when the store holds no lock.
pub(super) fn nothing_to_recover() -> StoreError {
    EncryptionError::Layout("no re-encryption is in progress".to_owned()).into()
}

/// Takes the lock with `journal` and `headers`, each `(folder, header)`:
/// writes them into a staging folder and renames that to `.rewrite/`.
///
/// # Errors
///
/// [`EncryptionError::Rewriting`] when the store is locked already, and the
/// filesystem's error otherwise; the staging folder is removed again.
pub(super) async fn publish<F: RemoteFs + ?Sized>(
    fs: &F,
    journal: &impl Serialize,
    headers: &[(&str, &StoreHeader)],
) -> Result<(), StoreError> {
    let lock = RemotePath::new(REWRITE_DIR)?;
    if fs.stat(&lock).await?.is_some() {
        return Err(EncryptionError::Rewriting { started: None }.into());
    }
    let token: [u8; 8] = random_bytes()?;
    let staged = RemotePath::new(&format!("{STAGING_PREFIX}{}", hex::encode(token)))?;
    fs.create_dir(&staged).await?;
    let json = serde_json::to_vec_pretty(journal).expect("the journal serialises");
    let published: Result<(), StoreError> = async {
        let path = staged.join(PLAN_FILE)?;
        let mut writer = fs.open_write(&path).await?;
        writer
            .write_all(&json)
            .await
            .map_err(|err| FsError::from_io(&path, err))?;
        writer
            .shutdown()
            .await
            .map_err(|err| FsError::from_io(&path, err))?;
        // Windows does not rename a folder that holds an open file.
        drop(writer);
        for (folder, header) in headers {
            write_header_in(fs, &staged.join(folder)?, header).await?;
        }
        fs.rename(&staged, &lock).await.map_err(|err| match err {
            FsError::AlreadyExists(_) => EncryptionError::Rewriting { started: None }.into(),
            err => err.into(),
        })
    }
    .await;
    if published.is_err()
        && let Err(clean) = fs.remove_dir_all(&staged).await
    {
        tracing::warn!(path = %staged, error = %clean, "could not remove a staged journal");
    }
    published
}

/// Removes journals staged by locks never taken, and then the lock.
pub(super) async fn release_lock<F: RemoteFs + ?Sized>(fs: &F) -> Result<(), StoreError> {
    sweep_staged(fs).await?;
    fs.remove_dir_all(&RemotePath::new(REWRITE_DIR)?).await?;
    Ok(())
}

/// Counts journals staged for locks that were never taken.
pub(super) async fn count_staged<F: RemoteFs + ?Sized>(fs: &F) -> Result<usize, StoreError> {
    Ok(fs
        .read_dir(&RemotePath::root())
        .await?
        .iter()
        .filter(|entry| entry.is_dir && entry.name.starts_with(STAGING_PREFIX))
        .count())
}

/// Removes journals staged by locks that were never taken.
pub(super) async fn sweep_staged<F: RemoteFs + ?Sized>(fs: &F) -> Result<(), StoreError> {
    for entry in fs.read_dir(&RemotePath::root()).await? {
        if entry.is_dir && entry.name.starts_with(STAGING_PREFIX) {
            fs.remove_dir_all(&RemotePath::new(&entry.name)?).await?;
        }
    }
    Ok(())
}

/// Releases a lock whose journal is missing or unreadable, as long as
/// nothing else was written under it.
///
/// # Errors
///
/// [`EncryptionError::Layout`], changing nothing, when the lock holds
/// anything but its journal.
pub(super) async fn release_unstarted<F: RemoteFs + ?Sized>(fs: &F) -> Result<(), StoreError> {
    let lock = RemotePath::new(REWRITE_DIR)?;
    let held: Vec<String> = fs
        .read_dir(&lock)
        .await?
        .into_iter()
        .map(|entry| entry.name)
        .filter(|name| name != PLAN_FILE && name != RECOVERY)
        .collect();
    if !held.is_empty() {
        return Err(EncryptionError::Layout(format!(
            "the lock's journal is missing or cannot be read, but the lock holds {}; nothing was changed. Back up the store folder before repairing it by hand",
            held.join(", ")
        ))
        .into());
    }
    release_lock(fs).await
}

/// A recovery that is running, or was interrupted.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RecoveryMarker {
    /// When it started, if the storage says.
    pub started: Option<DateTime<Utc>>,
}

impl RecoveryMarker {
    /// Whether it most likely stopped: it started [`RECOVERY_STALE_SECS`] or
    /// more before `now`, or at a time the storage does not say.
    pub fn is_stale(&self, now: DateTime<Utc>) -> bool {
        self.started
            .is_none_or(|at| now - at >= TimeDelta::seconds(RECOVERY_STALE_SECS))
    }
}

fn recovery_path() -> Result<RemotePath, FsError> {
    under_rewrite(RECOVERY)
}

/// The recovery running on the store, if any.
///
/// # Errors
///
/// The filesystem's error.
pub async fn recovery_in_progress<F: RemoteFs + ?Sized>(
    fs: &F,
) -> Result<Option<RecoveryMarker>, StoreError> {
    Ok(fs
        .stat(&recovery_path()?)
        .await?
        .map(|meta| RecoveryMarker {
            started: meta.modified,
        }))
}

/// Takes over from a recovery that stopped, so another can start. Only for
/// a recovery [`RecoveryMarker::is_stale`] says stopped.
///
/// # Errors
///
/// The filesystem's error.
pub async fn release_recovery<F: RemoteFs + ?Sized>(fs: &F) -> Result<(), StoreError> {
    fs.remove_dir_all(&recovery_path()?).await?;
    Ok(())
}

/// Claims the store's lock for a recovery.
///
/// # Errors
///
/// [`EncryptionError::Layout`] when the store is not locked, and
/// [`EncryptionError::Recovering`] when another recovery claimed it.
pub(super) async fn claim_recovery<F: RemoteFs + ?Sized>(fs: &F) -> Result<(), StoreError> {
    if fs.stat(&RemotePath::new(REWRITE_DIR)?).await?.is_none() {
        return Err(nothing_to_recover());
    }
    match fs.create_dir(&recovery_path()?).await {
        Ok(()) => Ok(()),
        Err(FsError::AlreadyExists(_)) => {
            let started = recovery_in_progress(fs)
                .await?
                .and_then(|marker| marker.started)
                .map(|at| at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
            Err(EncryptionError::Recovering { started }.into())
        }
        Err(FsError::NotFound(_)) => Err(nothing_to_recover()),
        Err(err) => Err(err.into()),
    }
}

/// Gives up the claim after a recovery failed, so the next one can start
/// at once; after a success the lock, claim included, is gone already.
pub(super) async fn unclaim<F: RemoteFs + ?Sized, T>(
    fs: &F,
    result: Result<T, StoreError>,
) -> Result<T, StoreError> {
    if result.is_err()
        && let Err(err) = release_recovery(fs).await
    {
        tracing::warn!(error = %err, "could not release the recovery claim");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encryption::RewriteKind;

    #[test]
    fn journals_parse_by_kind() {
        let rewrite =
            br#"{"kind":"rotate","started_at":"t","from_key":"a","to_key":"b","items":2}"#;
        assert!(matches!(
            parse(rewrite),
            Journal::Rewrite(RewritePlan {
                kind: RewriteKind::Rotate,
                items: 2,
                ..
            })
        ));
        let words = br#"{"kind":"words","started_at":"t","key":"k"}"#;
        assert_eq!(
            parse(words),
            Journal::HeaderChange(HeaderChange::new(
                HeaderChangeKind::Words,
                "t".to_owned(),
                "k".to_owned()
            ))
        );
        assert_eq!(
            parse(br#"{"kind":"shuffle"}"#),
            Journal::Newer {
                kind: "shuffle".to_owned()
            }
        );
        for bad in [
            &b""[..],
            b"{\"kind\":\"mig",
            b"{}",
            br#"{"kind":"migrate"}"#,
        ] {
            assert!(
                matches!(parse(bad), Journal::Unreadable { .. }),
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
    }

    #[test]
    fn a_header_change_serialises_with_its_kind() {
        let json = serde_json::to_string(&HeaderChange::new(
            HeaderChangeKind::FreshStart,
            "t".to_owned(),
            "k".to_owned(),
        ))
        .unwrap();
        assert!(json.contains(r#""kind":"fresh-start""#), "{json}");
    }

    #[test]
    fn a_recovery_is_stale_after_ten_minutes_or_at_an_unknown_start() {
        let now = DateTime::parse_from_rfc3339("2026-09-15T10:00:00Z")
            .unwrap()
            .to_utc();
        let at = |secs| RecoveryMarker {
            started: Some(now - TimeDelta::seconds(secs)),
        };
        assert!(!at(RECOVERY_STALE_SECS - 1).is_stale(now));
        assert!(at(RECOVERY_STALE_SECS).is_stale(now));
        assert!(RecoveryMarker { started: None }.is_stale(now));
    }
}
