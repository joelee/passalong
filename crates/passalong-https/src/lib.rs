//! passalong-server storage backend for `passalong`: the `https` kind.
//!
//! It is written from the server's published API documents alone; the
//! server is AGPL-3.0-or-later and no code of it is used here. Like the SSH
//! backend, it is kept free of CLI dependencies so GUI and Android
//! front-ends can reuse it.

pub mod admin;
pub mod api;
pub mod client;
pub mod error;
pub mod store;
pub mod tls;

use std::sync::Arc;

use passalong_core::api_key::load_api_key;
use passalong_core::clock::SystemClock;
use passalong_core::config::Config;
use passalong_core::crypto::KeyId;
use passalong_core::encryption::{Claim, EncryptionError, SystemGit, load_key_file, opening};
use passalong_core::store::{AdminFuture, BackendFuture, BackendRegistry, Store, StoreError};

pub use admin::HttpEncryptionAdmin;
pub use client::Client;
pub use error::{Code, HttpsError, Problem};
pub use store::{HttpStore, Partition, store_error};

use crate::api::EncryptionState;

/// Adds the `https` kind to `registry`.
pub fn register(registry: &mut BackendRegistry) {
    registry.register("https", opener);
    registry.register_admin("https", admin_opener);
}

fn admin_opener(config: &Config) -> AdminFuture<'_> {
    Box::pin(async move {
        let admin = HttpEncryptionAdmin::new(connect(config)?, Arc::new(SystemClock));
        Ok(Box::new(admin) as Box<dyn passalong_core::encryption::EncryptionAdmin>)
    })
}

fn opener(config: &Config) -> BackendFuture<'_> {
    Box::pin(async move { Ok(Box::new(open_https_store(config).await?) as Box<dyn Store>) })
}

/// A client of the server `config` names, with its API key.
///
/// # Errors
///
/// [`StoreError::Config`] without a `[server.https]` section or an API key,
/// the key file's refusal, and the transport's error.
pub fn connect(config: &Config) -> Result<Arc<Client>, StoreError> {
    let https =
        config.server.https.as_ref().ok_or_else(|| {
            StoreError::Config("the `server.https` section is missing".to_owned())
        })?;
    let key = load_api_key(&https.api_key_file, &SystemGit::new())
        .map_err(EncryptionError::from)?
        .ok_or_else(|| {
            StoreError::Config(format!(
                "no API key in {}: run `passalong init`, or put the key the server's operator gave you there",
                https.api_key_file.display()
            ))
        })?;
    Ok(Arc::new(Client::new(https, key).map_err(store_error)?))
}

/// What the server says about the workspace's encryption.
///
/// # Errors
///
/// [`StoreError::Backend`] when it names a state or key id this client
/// cannot read.
pub fn claim(encryption: &api::Encryption) -> Result<Claim, StoreError> {
    let key_id = || {
        let id = encryption.key_id.as_deref().unwrap_or_default();
        KeyId::parse(id).map_err(|_| {
            StoreError::Backend(format!(
                "the server names the key id `{id}`, which is not one"
            ))
        })
    };
    Ok(match encryption.state {
        EncryptionState::Plaintext => Claim::Plain,
        EncryptionState::Sealed => Claim::Encrypted(key_id()?),
        // The server tells when a rewrite's lease ends, not when it began.
        EncryptionState::Rewriting => Claim::Rewriting { started: None },
        _ => {
            return Err(StoreError::Backend(
                "the server reports an encryption state this version of passalong does not know"
                    .to_owned(),
            ));
        }
    })
}

/// Opens the workspace `config` names for its device: sealed with the
/// device's key, or plaintext. A device holding a key never opens a
/// workspace as plaintext, whatever the server says.
///
/// # Errors
///
/// As [`connect`], the refusals of `opening`, and the server's error.
pub async fn open_https_store(config: &Config) -> Result<HttpStore, StoreError> {
    let client = connect(config)?;
    let workspace = client.workspace().await.map_err(store_error)?;
    let key = match &config.client.key_file {
        Some(path) => load_key_file(path, &SystemGit::new()).map_err(EncryptionError::from)?,
        None => None,
    };
    let sealer = opening(claim(&workspace.encryption)?, key)?;
    tracing::debug!(workspace = %workspace.name, sealed = sealer.is_some(), "opened a server workspace");
    Ok(HttpStore::new(client, sealer, Arc::new(SystemClock)))
}

/// Version of this crate, shared by every crate in the workspace.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_the_workspace_version() {
        assert_eq!(super::VERSION, "0.3.0");
    }
}
