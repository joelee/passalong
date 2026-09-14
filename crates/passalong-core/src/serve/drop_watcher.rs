//! Finding files in the drop folder that are ready to send.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use notify::event::{AccessKind, AccessMode};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tokio::time::Instant;

/// Suffixes of files still being written by browsers and other tools.
const IGNORED_SUFFIXES: [&str; 4] = [".part", ".crdownload", ".tmp", ".passalong-part"];

/// A regular file in the drop folder, as last seen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileState {
    /// Full path.
    pub path: PathBuf,
    /// Size in bytes.
    pub size: u64,
    /// Last modification time.
    pub modified: SystemTime,
}

/// Whether a drop-folder entry should never be sent: hidden files and files
/// that are still being downloaded or written.
pub fn is_ignored(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    name.starts_with('.')
        || IGNORED_SUFFIXES
            .iter()
            .any(|suffix| lower.ends_with(suffix))
}

/// Lists the drop folder's regular files that are not ignored, sorted by
/// path. Directories (including `sent`) and symbolic links are skipped.
///
/// # Errors
///
/// When the folder cannot be read.
pub async fn scan(folder: &Path) -> io::Result<Vec<FileState>> {
    let mut dir = tokio::fs::read_dir(folder).await?;
    let mut files = Vec::new();
    while let Some(entry) = dir.next_entry().await? {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if is_ignored(name) {
            continue;
        }
        // The file may vanish between listing and inspecting it.
        let Ok(meta) = entry.metadata().await else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        files.push(FileState {
            path: entry.path(),
            size: meta.len(),
            modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

#[derive(Debug)]
struct Candidate {
    size: u64,
    modified: SystemTime,
    since: Instant,
}

/// Decides when a file has stopped changing. A file is ready once two scans
/// at least `stable_wait` apart show the same size and modification time;
/// from then on it is "in flight" and never reported again while it stays in
/// the folder.
#[derive(Debug)]
pub struct DropTracker {
    stable_wait: Duration,
    candidates: HashMap<PathBuf, Candidate>,
    in_flight: HashMap<PathBuf, InFlight>,
}

/// A file handed to the uploader, with the state it had then.
#[derive(Debug)]
struct InFlight {
    size: u64,
    modified: SystemTime,
    skipped: bool,
}

impl DropTracker {
    /// A tracker with nothing seen yet.
    pub fn new(stable_wait: Duration) -> Self {
        Self {
            stable_wait,
            candidates: HashMap::new(),
            in_flight: HashMap::new(),
        }
    }

    /// Records that the uploader skipped `path` for a local reason, such as
    /// missing permissions. The file is offered again once it changes.
    pub fn mark_skipped(&mut self, path: &Path) {
        if let Some(sent) = self.in_flight.get_mut(path) {
            sent.skipped = true;
        }
    }

    /// Whether some files are still settling.
    pub fn has_pending(&self) -> bool {
        !self.candidates.is_empty()
    }

    /// Takes the latest scan and returns the files that just became ready.
    /// Files missing from the scan are forgotten, so a new file reusing a
    /// sent file's name is tracked from scratch.
    pub fn observe(&mut self, files: Vec<FileState>, now: Instant) -> Vec<PathBuf> {
        let present: HashSet<&PathBuf> = files.iter().map(|file| &file.path).collect();
        self.in_flight.retain(|path, _| present.contains(path));
        self.candidates.retain(|path, _| present.contains(path));
        let mut ready = Vec::new();
        for file in files {
            if let Some(sent) = self.in_flight.get(&file.path) {
                let changed = sent.size != file.size || sent.modified != file.modified;
                if !(sent.skipped && changed) {
                    continue;
                }
                // A skipped file that changed may have been fixed: settle it again.
                self.in_flight.remove(&file.path);
            }
            match self.candidates.get_mut(&file.path) {
                Some(seen) if seen.size == file.size && seen.modified == file.modified => {
                    if now.duration_since(seen.since) >= self.stable_wait {
                        self.candidates.remove(&file.path);
                        self.in_flight.insert(
                            file.path.clone(),
                            InFlight {
                                size: file.size,
                                modified: file.modified,
                                skipped: false,
                            },
                        );
                        ready.push(file.path);
                    }
                }
                Some(seen) => {
                    seen.size = file.size;
                    seen.modified = file.modified;
                    seen.since = now;
                }
                None => {
                    self.candidates.insert(
                        file.path,
                        Candidate {
                            size: file.size,
                            modified: file.modified,
                            since: now,
                        },
                    );
                }
            }
        }
        ready
    }
}

/// A path in `dir` for `name` that does not exist yet: `name`, then
/// `stem (1).ext`, `stem (2).ext`, and so on.
pub fn unique_target(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(dot) if dot > 0 => (&name[..dot], Some(&name[dot + 1..])),
        _ => (name, None),
    };
    (1_u32..)
        .map(|n| {
            dir.join(match ext {
                Some(ext) => format!("{stem} ({n}).{ext}"),
                None => format!("{stem} ({n})"),
            })
        })
        .find(|candidate| !candidate.exists())
        .unwrap_or(first)
}

/// Watches `folder` (not its subfolders) and signals `changed` when an event
/// may mean its files changed (see [`is_change`]). Events are only hints to
/// scan again; a full channel already means a scan is due, so extra events
/// are dropped.
///
/// # Errors
///
/// When the operating system refuses the watch, for example because the
/// inotify watch limit is reached.
pub fn watch_folder(
    folder: &Path,
    changed: mpsc::Sender<()>,
) -> notify::Result<RecommendedWatcher> {
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if event.is_ok_and(|event| is_change(&event.kind)) {
            let _ = changed.try_send(());
        }
    })?;
    watcher.watch(folder, RecursiveMode::NonRecursive)?;
    Ok(watcher)
}

/// Whether a watcher event may mean the folder's files changed. Opening,
/// reading, and closing without writing are not changes, and `scan` itself
/// opens the folder: reacting to those made every scan trigger the next one
/// and kept `serve` busy while idle. Closing a file after writing is a
/// change, since the file may now be complete.
fn is_change(kind: &EventKind) -> bool {
    match kind {
        EventKind::Access(AccessKind::Close(AccessMode::Write)) => true,
        EventKind::Access(_) => false,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{Duration, UNIX_EPOCH};
    use tempfile::TempDir;
    use tokio::time::Instant;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn state(name: &str, size: u64, mtime: u64) -> FileState {
        FileState {
            path: PathBuf::from("/drop").join(name),
            size,
            modified: UNIX_EPOCH + Duration::from_secs(mtime),
        }
    }

    #[test]
    fn files_become_ready_after_staying_unchanged_for_the_wait() {
        let start = Instant::now();
        let mut tracker = DropTracker::new(ms(1000));
        assert!(!tracker.has_pending());
        assert!(tracker.observe(vec![state("a", 1, 10)], start).is_empty());
        assert!(tracker.has_pending());
        assert!(
            tracker
                .observe(vec![state("a", 1, 10)], start + ms(500))
                .is_empty()
        );
        assert_eq!(
            tracker.observe(vec![state("a", 1, 10)], start + ms(1000)),
            [PathBuf::from("/drop/a")]
        );
        assert!(!tracker.has_pending());
        assert!(
            tracker
                .observe(vec![state("a", 1, 10)], start + ms(9000))
                .is_empty(),
            "in flight: not again"
        );
    }

    #[test]
    fn size_or_time_changes_restart_the_wait() {
        let start = Instant::now();
        let mut tracker = DropTracker::new(ms(1000));
        tracker.observe(vec![state("a", 1, 10)], start);
        assert!(
            tracker
                .observe(vec![state("a", 2, 10)], start + ms(900))
                .is_empty(),
            "grew"
        );
        assert!(
            tracker
                .observe(vec![state("a", 2, 11)], start + ms(1800))
                .is_empty(),
            "touched"
        );
        assert!(
            tracker
                .observe(vec![state("a", 2, 11)], start + ms(2700))
                .is_empty()
        );
        assert_eq!(
            tracker
                .observe(vec![state("a", 2, 11)], start + ms(2800))
                .len(),
            1
        );
    }

    #[test]
    fn zero_wait_still_needs_two_identical_scans() {
        let start = Instant::now();
        let mut tracker = DropTracker::new(Duration::ZERO);
        assert!(tracker.observe(vec![state("a", 1, 10)], start).is_empty());
        assert_eq!(tracker.observe(vec![state("a", 1, 10)], start).len(), 1);
    }

    #[test]
    fn files_that_disappear_are_forgotten() {
        let start = Instant::now();
        let mut tracker = DropTracker::new(ms(100));
        tracker.observe(vec![state("a", 1, 10), state("b", 1, 10)], start);
        assert_eq!(
            tracker
                .observe(vec![state("a", 1, 10), state("b", 1, 10)], start + ms(100))
                .len(),
            2
        );
        // `a` was sent and moved away; `b` is still being retried.
        assert!(
            tracker
                .observe(vec![state("b", 1, 10)], start + ms(200))
                .is_empty()
        );
        // A new file called `a` arrives later and is tracked from scratch.
        assert!(
            tracker
                .observe(vec![state("a", 5, 20), state("b", 1, 10)], start + ms(300))
                .is_empty()
        );
        assert_eq!(
            tracker.observe(vec![state("a", 5, 20), state("b", 1, 10)], start + ms(400)),
            [PathBuf::from("/drop/a")]
        );
    }

    #[test]
    fn skipped_files_are_offered_again_once_they_change() {
        let start = Instant::now();
        let mut tracker = DropTracker::new(ms(100));
        tracker.observe(vec![state("a", 1, 10)], start);
        assert_eq!(
            tracker
                .observe(vec![state("a", 1, 10)], start + ms(100))
                .len(),
            1
        );
        tracker.mark_skipped(std::path::Path::new("/drop/a"));
        tracker.mark_skipped(std::path::Path::new("/drop/never-seen"));
        assert!(
            tracker
                .observe(vec![state("a", 1, 10)], start + ms(5_000))
                .is_empty(),
            "unchanged: stays skipped"
        );
        assert!(
            tracker
                .observe(vec![state("a", 1, 11)], start + ms(5_100))
                .is_empty(),
            "changed: settles again"
        );
        assert_eq!(
            tracker.observe(vec![state("a", 1, 11)], start + ms(5_200)),
            [PathBuf::from("/drop/a")]
        );
    }

    #[test]
    fn files_in_flight_are_not_offered_again_even_if_they_change() {
        let start = Instant::now();
        let mut tracker = DropTracker::new(ms(100));
        tracker.observe(vec![state("b", 1, 10)], start);
        assert_eq!(
            tracker
                .observe(vec![state("b", 1, 10)], start + ms(100))
                .len(),
            1
        );
        assert!(
            tracker
                .observe(vec![state("b", 2, 12)], start + ms(5_000))
                .is_empty()
        );
        assert!(
            tracker
                .observe(vec![state("b", 2, 12)], start + ms(9_000))
                .is_empty()
        );
    }

    #[test]
    fn partial_hidden_and_temporary_names_are_ignored() {
        for name in [
            ".hidden",
            "x.part",
            "x.crdownload",
            "x.tmp",
            "x.passalong-part",
            "BIG.PART",
        ] {
            assert!(is_ignored(name), "{name}");
        }
        for name in ["a.txt", "part", "tmp.txt", "report.partial.pdf"] {
            assert!(!is_ignored(name), "{name}");
        }
    }

    #[test]
    fn sent_names_get_a_number_on_collision() {
        let dir = TempDir::new().unwrap();
        let sent = dir.path();
        assert_eq!(unique_target(sent, "a.txt"), sent.join("a.txt"));
        std::fs::write(sent.join("a.txt"), b"").unwrap();
        assert_eq!(unique_target(sent, "a.txt"), sent.join("a (1).txt"));
        std::fs::write(sent.join("a (1).txt"), b"").unwrap();
        assert_eq!(unique_target(sent, "a.txt"), sent.join("a (2).txt"));
        std::fs::write(sent.join("noext"), b"").unwrap();
        assert_eq!(unique_target(sent, "noext"), sent.join("noext (1)"));
        std::fs::write(sent.join("archive.tar.gz"), b"").unwrap();
        assert_eq!(
            unique_target(sent, "archive.tar.gz"),
            sent.join("archive.tar (1).gz")
        );
    }

    /// Waits up to `wait` for a notification from the watcher; false when
    /// none came.
    async fn notified(rx: &mut mpsc::Receiver<()>, wait: Duration) -> bool {
        matches!(tokio::time::timeout(wait, rx.recv()).await, Ok(Some(())))
    }

    /// Discards notifications that arrive within `wait`.
    async fn drain(rx: &mut mpsc::Receiver<()>, wait: Duration) {
        while notified(rx, wait).await {}
    }

    #[tokio::test]
    async fn scanning_the_folder_does_not_trigger_the_watcher() {
        // Opening the folder to list it, and reading a file, are not
        // changes: reacting to them made every scan trigger the next.
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"abc").unwrap();
        let (tx, mut rx) = mpsc::channel(16);
        let _watcher = watch_folder(dir.path(), tx).unwrap();
        drain(&mut rx, ms(200)).await;
        scan(dir.path()).await.unwrap();
        std::fs::read(dir.path().join("a.txt")).unwrap();
        assert!(
            !notified(&mut rx, ms(500)).await,
            "a scan or a read notified the watcher"
        );
    }

    #[tokio::test]
    async fn creating_writing_renaming_and_removing_files_trigger_the_watcher() {
        /// Asserts a notification arrives for `step`, then discards the rest.
        async fn expect(rx: &mut mpsc::Receiver<()>, step: &str) {
            // FSEvents on macOS can take a moment; inotify is immediate.
            assert!(notified(rx, ms(5_000)).await, "{step} did not notify");
            drain(rx, ms(200)).await;
        }
        let dir = TempDir::new().unwrap();
        let (tx, mut rx) = mpsc::channel(16);
        let _watcher = watch_folder(dir.path(), tx).unwrap();
        drain(&mut rx, ms(200)).await;
        let (a, b) = (dir.path().join("a.txt"), dir.path().join("b.txt"));
        std::fs::write(&a, b"").unwrap();
        expect(&mut rx, "create").await;
        {
            use std::io::Write as _;
            let mut file = std::fs::OpenOptions::new().append(true).open(&a).unwrap();
            file.write_all(b"more").unwrap();
        }
        expect(&mut rx, "write").await;
        std::fs::rename(&a, &b).unwrap();
        expect(&mut rx, "rename").await;
        std::fs::remove_file(&b).unwrap();
        expect(&mut rx, "remove").await;
    }

    #[test]
    fn only_events_that_may_change_files_count() {
        use notify::event::{CreateKind, ModifyKind, RemoveKind, RenameMode};
        for kind in [
            EventKind::Create(CreateKind::File),
            EventKind::Modify(ModifyKind::Any),
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
            EventKind::Remove(RemoveKind::File),
            EventKind::Access(AccessKind::Close(AccessMode::Write)),
            EventKind::Other,
        ] {
            assert!(is_change(&kind), "{kind:?}");
        }
        for kind in [
            EventKind::Access(AccessKind::Open(AccessMode::Any)),
            EventKind::Access(AccessKind::Close(AccessMode::Read)),
            EventKind::Access(AccessKind::Read),
            EventKind::Access(AccessKind::Any),
        ] {
            assert!(!is_change(&kind), "{kind:?}");
        }
    }

    #[tokio::test]
    async fn scanning_lists_only_regular_files_worth_sending() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"abc").unwrap();
        std::fs::write(dir.path().join(".hidden"), b"").unwrap();
        std::fs::write(dir.path().join("big.part"), b"").unwrap();
        std::fs::create_dir(dir.path().join("sent")).unwrap();
        let files = scan(dir.path()).await.unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, dir.path().join("a.txt"));
        assert_eq!(files[0].size, 3);
        assert!(scan(&dir.path().join("missing")).await.is_err());
    }
}
