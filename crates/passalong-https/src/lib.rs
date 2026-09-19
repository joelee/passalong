//! passalong-server storage backend for `passalong`: the `https` kind.
//!
//! It is written from the server's published API documents alone; the
//! server is AGPL-3.0-or-later and no code of it is used here. Like the SSH
//! backend, it is kept free of CLI dependencies so GUI and Android
//! front-ends can reuse it.

pub mod api;
pub mod client;
pub mod error;
pub mod tls;

pub use client::Client;
pub use error::{Code, HttpsError, Problem};

/// Version of this crate, shared by every crate in the workspace.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_the_workspace_version() {
        assert_eq!(super::VERSION, "0.2.1");
    }
}
