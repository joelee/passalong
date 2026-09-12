//! SSH/SFTP storage backend for `passalong`.
//!
//! The backend implementation lands in PLAN-00001 STEP-11; this crate is kept
//! free of CLI dependencies so GUI and Android front-ends can reuse it.

/// Version of this crate, shared by every crate in the workspace.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_the_workspace_version() {
        assert_eq!(super::VERSION, "0.1.0");
    }
}
