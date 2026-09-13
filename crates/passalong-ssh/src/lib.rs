//! SSH/SFTP storage backend for `passalong`.
//!
//! This crate is kept free of CLI dependencies so GUI and Android front-ends
//! can reuse it.

pub mod backend;
pub mod connect;
pub mod error;
pub mod host_key;
pub mod sftp_fs;

pub use backend::{open_ssh_store, register};
pub use connect::fetch_host_key;
pub use host_key::DiscoveredKey;

/// Version of this crate, shared by every crate in the workspace.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_the_workspace_version() {
        assert_eq!(super::VERSION, "0.1.2");
    }
}
