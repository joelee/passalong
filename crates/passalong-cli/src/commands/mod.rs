//! One module per subcommand. Each takes its dependencies explicitly (the
//! store, the output stream, the clipboard) so it can be tested with
//! in-memory doubles.

pub mod clipboard;
pub mod delete;
pub mod file;
pub mod init;
pub mod list;
pub mod load;
pub mod prune;
pub mod serve;

/// Helpers shared by the command tests.
#[cfg(test)]
pub(crate) mod support {
    use std::sync::Arc;

    use passalong_core::fs::LocalFs;
    use passalong_core::random::StdRandom;
    use passalong_core::store::FsStore;
    use passalong_core::testing::ManualClock;
    use tempfile::TempDir;

    pub const T: &str = "2026-09-12T09:53:11Z";

    /// A local store in a temporary directory with a clock tests can move.
    pub struct TestStore {
        /// The store\'s root directory.
        pub dir: TempDir,
        pub clock: Arc<ManualClock>,
        pub store: FsStore<LocalFs>,
    }

    impl TestStore {
        pub fn new() -> Self {
            let dir = TempDir::new().unwrap();
            let clock = Arc::new(ManualClock::at(T));
            let store = FsStore::new(
                LocalFs::new(dir.path()),
                clock.clone(),
                Box::new(StdRandom::new()),
            );
            Self { dir, clock, store }
        }
    }

    pub fn bytes(data: &[u8]) -> passalong_core::fs::BoxRead {
        Box::new(std::io::Cursor::new(data.to_vec()))
    }
}
