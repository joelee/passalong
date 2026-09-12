//! Core library for `passalong`, a lightweight cross-platform clipboard and
//! file sharing tool.
//!
//! This crate holds everything a front-end needs that is independent of the
//! command line: configuration, the item model, storage traits, and
//! telemetry. It must never depend on CLI or terminal crates so that future
//! GUI and Android front-ends can reuse it.

pub mod clock;
pub mod config;
pub mod model;
pub mod random;
pub mod telemetry;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

/// Version of this crate, shared by every crate in the workspace.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_the_workspace_version() {
        assert_eq!(super::VERSION, "0.1.0");
    }
}
