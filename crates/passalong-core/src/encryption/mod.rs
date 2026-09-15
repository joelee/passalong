//! Encrypted stores on the client side: the key file that holds a device's
//! copy of a store's data key.
//!
//! The keys and formats themselves live in [`crate::crypto`].

mod key_file;

pub use key_file::{GitCheck, KeyFileError, SystemGit, load_key_file, save_key_file};
