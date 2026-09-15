//! Encrypted stores on the client side: the store header, the key file that
//! holds a device's copy of a store's data key, and opening a store as
//! plaintext or sealed, as its header says.
//!
//! The keys and formats themselves live in [`crate::crypto`].
//!
//! # Layout
//!
//! An encrypted store's root holds:
//!
//! - `encryption/header.json`, the [`StoreHeader`], in a folder of its own:
//!   a rename never replaces a file, so the header is replaced by swapping
//!   folders;
//! - `items`, a regular file ([`STOP_TEXT`]) where clients before v0.2.0
//!   expect their item folder, so their commands fail instead of storing
//!   plaintext;
//! - `v2/items/` and `v2/tmp/`, the sealed items and their staging;
//! - `plain/items/`, only after a fresh start, for the items stored before;
//! - `.rewrite/`, only while items are re-encrypted; creating it is the lock.

mod admin;
pub(crate) mod header;
mod key_file;
mod open;
mod rewrite;

pub use admin::{
    StoreState, change_words, fresh_start, inspect, join, plain_store, remove_plain_if_empty,
    set_up,
};
pub use header::{StoreHeader, create_header, read_header, replace_header, write_stop_file};
pub use key_file::{
    GitCheck, KeyFileError, SystemGit, check_key_location, load_key_file, save_key_file,
};
pub use open::{open_store, open_with_key};
pub use rewrite::{RewriteKind, RewritePlan, finish, migrate, read_plan, rotate, undo};

use crate::crypto::CryptoError;

/// The folder holding the store header.
pub const ENCRYPTION_DIR: &str = "encryption";
/// The store header's file name inside [`ENCRYPTION_DIR`].
pub const HEADER_FILE: &str = "header.json";
/// The file that stops clients before v0.2.0, where they expect `items/`.
pub const STOP_FILE: &str = "items";
/// What [`STOP_FILE`] says.
pub const STOP_TEXT: &str = "This store is encrypted. Upgrade to passalong 0.2.0 or later.\n";
/// The folder that locks the store while its items are re-encrypted.
pub const REWRITE_DIR: &str = ".rewrite";
/// The folder holding the plaintext items a fresh start kept.
pub const PLAIN_DIR: &str = "plain";
/// Where a sealed store stages uploads, and the header while it is replaced.
pub(crate) const SEALED_TMP_DIR: &str = "v2/tmp";

/// Why an encrypted store, or an item in it, cannot be used. Every message
/// names what to do about it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum EncryptionError {
    /// A recent item whose sealed data does not open: its files are most
    /// likely still arriving, for example through a synced folder.
    #[error(
        "item {id} is not complete yet; if the store is in a synced folder, it may still be syncing"
    )]
    Incomplete {
        /// The item's id.
        id: String,
    },
    /// Sealing or opening failed.
    #[error(transparent)]
    Crypto(#[from] CryptoError),
    /// The device's key file cannot be used.
    #[error(transparent)]
    KeyFile(#[from] KeyFileError),
    /// The store is encrypted, and this device has no key.
    #[error(
        "the store is encrypted and this device has no key for it; run `passalong encrypt --join`"
    )]
    NoKey,
    /// The device's key is not the store's.
    #[error(
        "this device's key {device} is not the store's key {store}; run `passalong encrypt --join` with the store's current words"
    )]
    KeyMismatch {
        /// Short id of the device's key.
        device: String,
        /// Short id of the store's key.
        store: String,
    },
    /// The device has a key, but the store is not encrypted.
    #[error(
        "this device has an encryption key, but the store is not encrypted; run `passalong encrypt` to encrypt it, or delete the key file (client.key_file)"
    )]
    KeyWithoutEncryption,
    /// The store is not encrypted.
    #[error("the store is not encrypted; run `passalong encrypt` to encrypt it")]
    NotEncrypted,
    /// Another client is re-encrypting the store's items.
    #[error(
        "the store's items are being re-encrypted{}; wait for that to finish, or run `passalong encrypt --recover`",
        since(.started)
    )]
    Rewriting {
        /// When the re-encryption started, if known.
        started: Option<String>,
    },
    /// The store's items are encrypted, but its header is gone.
    #[error("the store's encryption header is missing; run `passalong encrypt --recover`")]
    HeaderMissing,
    /// The store's key changed after this client opened it.
    #[error(
        "the store's key changed while this command ran; run `passalong encrypt --join` with the new words"
    )]
    KeyChanged,
    /// Encryption was set up on a store that already has it.
    #[error("the store is already encrypted")]
    AlreadyEncrypted,
    /// The store header cannot be read.
    #[error("the store header is not valid: {0}")]
    Header(String),
    /// The store's files are not laid out as expected.
    #[error("the store's layout is unexpected: {0}")]
    Layout(String),
}

fn since(started: &Option<String>) -> String {
    started
        .as_ref()
        .map(|at| format!(" (since {at})"))
        .unwrap_or_default()
}
