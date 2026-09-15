//! Encrypted stores on the client side: the key file that holds a device's
//! copy of a store's data key.
//!
//! The keys and formats themselves live in [`crate::crypto`].

mod key_file;

pub use key_file::{GitCheck, KeyFileError, SystemGit, load_key_file, save_key_file};

use crate::crypto::CryptoError;

/// Why an encrypted store, or an item in it, cannot be used.
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
}
