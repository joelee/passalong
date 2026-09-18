//! Test doubles for passalong's external interfaces.
//!
//! Compiled for this crate's own tests and, behind the `testing` feature, for
//! other crates' tests. Never enable the feature in production builds.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tracing_subscriber::fmt::MakeWriter;

use crate::clipboard::{Clipboard, ClipboardError, RgbaImage};
use crate::clock::Clock;
use crate::config::EnvProvider;
use crate::fs::{BoxRead, BoxWrite, DirEntry, FsError, Metadata, RemoteFs, RemotePath};
use crate::random::RandomSource;

/// [`Clock`] that always returns the same instant.
#[derive(Debug, Clone, Copy)]
pub struct FixedClock(pub DateTime<Utc>);

impl FixedClock {
    /// Creates a clock fixed at an RFC 3339 timestamp.
    ///
    /// # Panics
    ///
    /// Panics if `rfc3339` is not a valid RFC 3339 timestamp.
    pub fn at(rfc3339: &str) -> Self {
        let instant = DateTime::parse_from_rfc3339(rfc3339).expect("valid RFC 3339 timestamp");
        Self(instant.with_timezone(&Utc))
    }
}

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

/// [`Clock`] that tests move forward explicitly.
#[derive(Debug)]
pub struct ManualClock(Mutex<DateTime<Utc>>);

impl ManualClock {
    /// Creates a clock at an RFC 3339 timestamp.
    ///
    /// # Panics
    ///
    /// Panics if `rfc3339` is not a valid RFC 3339 timestamp.
    pub fn at(rfc3339: &str) -> Self {
        Self(Mutex::new(FixedClock::at(rfc3339).0))
    }

    /// Moves the clock forward by `secs` seconds.
    pub fn advance(&self, secs: i64) {
        *self.0.lock().expect("clock lock") += chrono::TimeDelta::seconds(secs);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().expect("clock lock")
    }
}

/// [`RandomSource`] that replays a fixed sequence.
#[derive(Debug, Clone)]
pub struct SeqRandom {
    values: VecDeque<u64>,
}

impl SeqRandom {
    /// Creates a source that yields `values` in order.
    pub fn new(values: impl IntoIterator<Item = u64>) -> Self {
        Self {
            values: values.into_iter().collect(),
        }
    }
}

impl RandomSource for SeqRandom {
    /// # Panics
    ///
    /// Panics when the sequence is exhausted, which signals a test bug.
    fn next_u64(&mut self) -> u64 {
        self.values.pop_front().expect("SeqRandom exhausted")
    }
}

/// In-memory log sink usable as a `tracing` writer. Clones share one buffer.
#[derive(Debug, Clone, Default)]
pub struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    /// Returns everything written so far.
    ///
    /// # Panics
    ///
    /// Panics if the buffer is not UTF-8 or its lock is poisoned.
    pub fn contents(&self) -> String {
        let bytes = self.0.lock().expect("log buffer lock").clone();
        String::from_utf8(bytes).expect("log output is UTF-8")
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .expect("log buffer lock")
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for LogBuffer {
    type Writer = LogBuffer;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// [`EnvProvider`] backed by an in-memory map, so tests never read or modify
/// the real process environment.
#[derive(Debug, Clone)]
pub struct MapEnv {
    vars: HashMap<String, String>,
    hostname: String,
}

impl Default for MapEnv {
    fn default() -> Self {
        Self {
            vars: HashMap::new(),
            hostname: "test-host".to_owned(),
        }
    }
}

impl MapEnv {
    /// Creates an empty environment whose host name is `test-host`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns this environment with `key` set to `value`.
    #[must_use]
    pub fn with(mut self, key: &str, value: &str) -> Self {
        self.vars.insert(key.to_owned(), value.to_owned());
        self
    }

    /// Returns this environment with the given host name.
    #[must_use]
    pub fn with_hostname(mut self, hostname: &str) -> Self {
        self.hostname = hostname.to_owned();
        self
    }
}

impl EnvProvider for MapEnv {
    fn var(&self, key: &str) -> Option<String> {
        self.vars.get(key).cloned()
    }

    fn hostname(&self) -> String {
        self.hostname.clone()
    }
}

/// The [`RemoteFs`] operations [`FaultyFs`] can fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FsOp {
    /// [`RemoteFs::create_dir_all`].
    CreateDirAll,
    /// [`RemoteFs::read_dir`].
    ReadDir,
    /// [`RemoteFs::open_read`].
    OpenRead,
    /// [`RemoteFs::open_write`].
    OpenWrite,
    /// [`RemoteFs::rename`].
    Rename,
    /// [`RemoteFs::remove_dir_all`].
    RemoveDirAll,
    /// [`RemoteFs::stat`].
    Stat,
    /// [`RemoteFs::create_dir`].
    CreateDir,
    /// [`RemoteFs::remove_file`].
    RemoveFile,
}

#[derive(Debug, Default)]
struct FaultState {
    calls: HashMap<FsOp, usize>,
    failing: HashSet<(FsOp, usize)>,
    /// Bytes each chosen `open_write` call's writer takes before failing.
    cuts: HashMap<usize, usize>,
}

mod cut_writer {
    use std::io;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use tokio::io::AsyncWrite;

    use crate::fs::BoxWrite;

    /// A writer that takes `left` more bytes, then fails every write and
    /// its shutdown: a write cut short part-way.
    pub(super) struct CutWriter {
        pub(super) inner: BoxWrite,
        pub(super) left: usize,
    }

    fn cut() -> io::Error {
        io::Error::other("injected write failure")
    }

    impl AsyncWrite for CutWriter {
        fn poll_write(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<io::Result<usize>> {
            if self.left == 0 {
                return Poll::Ready(Err(cut()));
            }
            let this = &mut *self;
            let n = buf.len().min(this.left);
            match Pin::new(&mut this.inner).poll_write(cx, &buf[..n]) {
                Poll::Ready(Ok(written)) => {
                    this.left -= written;
                    Poll::Ready(Ok(written))
                }
                other => other,
            }
        }

        fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Pin::new(&mut self.inner).poll_flush(cx)
        }

        fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            // The bytes taken reach the file; the caller still sees a failure.
            match Pin::new(&mut self.inner).poll_shutdown(cx) {
                Poll::Ready(_) => Poll::Ready(Err(cut())),
                Poll::Pending => Poll::Pending,
            }
        }
    }
}

/// [`RemoteFs`] wrapper that fails chosen calls, for testing error paths
/// of code built on the filesystem layer.
#[derive(Debug)]
pub struct FaultyFs<F> {
    inner: F,
    state: Mutex<FaultState>,
}

impl<F> FaultyFs<F> {
    /// Wraps `inner`; nothing fails until configured.
    pub fn new(inner: F) -> Self {
        Self {
            inner,
            state: Mutex::default(),
        }
    }

    /// The wrapped filesystem.
    pub fn inner(&self) -> &F {
        &self.inner
    }

    /// Makes the `nth` call of `op` (counting from 1, over the wrapper's
    /// lifetime) fail.
    pub fn fail_nth(&self, op: FsOp, nth: usize) {
        self.state
            .lock()
            .expect("fault state lock")
            .failing
            .insert((op, nth));
    }

    /// Makes the next `times` calls of `op` fail.
    pub fn fail_next(&self, op: FsOp, times: usize) {
        let mut state = self.state.lock().expect("fault state lock");
        let done = state.calls.get(&op).copied().unwrap_or(0);
        state.failing.extend((1..=times).map(|i| (op, done + i)));
    }

    /// Makes the `nth` [`RemoteFs::open_write`] call (counting from 1)
    /// return a writer that takes `after` bytes and then fails every write
    /// and its shutdown, as when a connection drops part-way through.
    pub fn cut_nth_write(&self, nth: usize, after: usize) {
        self.state
            .lock()
            .expect("fault state lock")
            .cuts
            .insert(nth, after);
    }

    /// How many times `op` has been called.
    pub fn calls(&self, op: FsOp) -> usize {
        self.state
            .lock()
            .expect("fault state lock")
            .calls
            .get(&op)
            .copied()
            .unwrap_or(0)
    }

    /// Counts a call of `op`, fails it when chosen, and returns its number.
    fn check(&self, op: FsOp, path: &RemotePath) -> Result<usize, FsError> {
        let mut state = self.state.lock().expect("fault state lock");
        let call = state.calls.entry(op).or_insert(0);
        *call += 1;
        let key = (op, *call);
        if state.failing.remove(&key) {
            return Err(FsError::Other {
                path: path.to_string(),
                message: format!("injected {op:?} failure"),
            });
        }
        Ok(key.1)
    }
}

#[async_trait]
impl<F: RemoteFs> RemoteFs for FaultyFs<F> {
    async fn create_dir_all(&self, path: &RemotePath) -> Result<(), FsError> {
        self.check(FsOp::CreateDirAll, path)?;
        self.inner.create_dir_all(path).await
    }

    async fn read_dir(&self, path: &RemotePath) -> Result<Vec<DirEntry>, FsError> {
        self.check(FsOp::ReadDir, path)?;
        self.inner.read_dir(path).await
    }

    async fn open_read(&self, path: &RemotePath) -> Result<BoxRead, FsError> {
        self.check(FsOp::OpenRead, path)?;
        self.inner.open_read(path).await
    }

    async fn open_write(&self, path: &RemotePath) -> Result<BoxWrite, FsError> {
        let call = self.check(FsOp::OpenWrite, path)?;
        let writer = self.inner.open_write(path).await?;
        let cut = self
            .state
            .lock()
            .expect("fault state lock")
            .cuts
            .remove(&call);
        Ok(match cut {
            Some(left) => Box::new(cut_writer::CutWriter {
                inner: writer,
                left,
            }),
            None => writer,
        })
    }

    async fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<(), FsError> {
        self.check(FsOp::Rename, from)?;
        self.inner.rename(from, to).await
    }

    async fn remove_dir_all(&self, path: &RemotePath) -> Result<(), FsError> {
        self.check(FsOp::RemoveDirAll, path)?;
        self.inner.remove_dir_all(path).await
    }

    async fn stat(&self, path: &RemotePath) -> Result<Option<Metadata>, FsError> {
        self.check(FsOp::Stat, path)?;
        self.inner.stat(path).await
    }

    async fn create_dir(&self, path: &RemotePath) -> Result<(), FsError> {
        self.check(FsOp::CreateDir, path)?;
        self.inner.create_dir(path).await
    }

    async fn remove_file(&self, path: &RemotePath) -> Result<(), FsError> {
        self.check(FsOp::RemoveFile, path)?;
        self.inner.remove_file(path).await
    }
}

/// Sets the modification time of the file or folder at `path`. Windows
/// opens a folder only with a flag for it, which `File::open` does not set.
///
/// # Errors
///
/// When the path cannot be opened or its time set.
pub fn set_modified(path: &std::path::Path, time: std::time::SystemTime) -> io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_WRITE_ATTRIBUTES, and FILE_FLAG_BACKUP_SEMANTICS for folders.
        options.access_mode(0x0100).custom_flags(0x0200_0000);
    }
    #[cfg(not(windows))]
    options.read(true);
    options.open(path)?.set_modified(time)
}

#[derive(Debug, Default)]
struct PauseState {
    calls: HashMap<FsOp, usize>,
    at: Option<(FsOp, usize)>,
}

/// [`RemoteFs`] wrapper that holds one chosen call until the test lets it
/// go, for running two operations in a set order.
#[derive(Debug, Default)]
pub struct PausingFs<F> {
    inner: F,
    state: Mutex<PauseState>,
    reached: tokio::sync::Notify,
    released: tokio::sync::Notify,
}

impl<F> PausingFs<F> {
    /// Wraps `inner`; nothing is held until [`PausingFs::pause_at`].
    pub fn new(inner: F) -> Self {
        Self {
            inner,
            state: Mutex::default(),
            reached: tokio::sync::Notify::new(),
            released: tokio::sync::Notify::new(),
        }
    }

    /// Holds the `nth` call of `op`, counting from 1 from now, before it
    /// runs.
    pub fn pause_at(&self, op: FsOp, nth: usize) {
        let mut state = self.state.lock().expect("pause state lock");
        state.calls.clear();
        state.at = Some((op, nth));
    }

    /// Waits until the held call arrives.
    pub async fn reached(&self) {
        self.reached.notified().await;
    }

    /// Lets the held call run.
    pub fn release(&self) {
        self.released.notify_one();
    }

    async fn hold(&self, op: FsOp) {
        let held = {
            let mut state = self.state.lock().expect("pause state lock");
            let call = state.calls.entry(op).or_insert(0);
            *call += 1;
            let key = (op, *call);
            state.at == Some(key)
        };
        if held {
            self.reached.notify_one();
            self.released.notified().await;
        }
    }
}

#[async_trait]
impl<F: RemoteFs> RemoteFs for PausingFs<F> {
    async fn create_dir_all(&self, path: &RemotePath) -> Result<(), FsError> {
        self.hold(FsOp::CreateDirAll).await;
        self.inner.create_dir_all(path).await
    }

    async fn read_dir(&self, path: &RemotePath) -> Result<Vec<DirEntry>, FsError> {
        self.hold(FsOp::ReadDir).await;
        self.inner.read_dir(path).await
    }

    async fn open_read(&self, path: &RemotePath) -> Result<BoxRead, FsError> {
        self.hold(FsOp::OpenRead).await;
        self.inner.open_read(path).await
    }

    async fn open_write(&self, path: &RemotePath) -> Result<BoxWrite, FsError> {
        self.hold(FsOp::OpenWrite).await;
        self.inner.open_write(path).await
    }

    async fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<(), FsError> {
        self.hold(FsOp::Rename).await;
        self.inner.rename(from, to).await
    }

    async fn remove_dir_all(&self, path: &RemotePath) -> Result<(), FsError> {
        self.hold(FsOp::RemoveDirAll).await;
        self.inner.remove_dir_all(path).await
    }

    async fn stat(&self, path: &RemotePath) -> Result<Option<Metadata>, FsError> {
        self.hold(FsOp::Stat).await;
        self.inner.stat(path).await
    }

    async fn create_dir(&self, path: &RemotePath) -> Result<(), FsError> {
        self.hold(FsOp::CreateDir).await;
        self.inner.create_dir(path).await
    }

    async fn remove_file(&self, path: &RemotePath) -> Result<(), FsError> {
        self.hold(FsOp::RemoveFile).await;
        self.inner.remove_file(path).await
    }
}

#[derive(Debug, Default)]
struct MockClipboardState {
    reads: VecDeque<Result<Option<String>, ClipboardError>>,
    current: Option<String>,
    writes: Vec<String>,
    fail_writes: bool,
    image_reads: VecDeque<Option<RgbaImage>>,
    current_image: Option<RgbaImage>,
    image_writes: Vec<RgbaImage>,
}

/// In-memory [`Clipboard`]. Clones share state, so a test can keep one
/// handle to inspect writes after moving another into the code under test.
#[derive(Debug, Clone, Default)]
pub struct MockClipboard(Arc<Mutex<MockClipboardState>>);

impl MockClipboard {
    /// An empty clipboard.
    pub fn new() -> Self {
        Self::default()
    }

    /// A clipboard currently holding `text`.
    pub fn with_text(text: &str) -> Self {
        let mock = Self::new();
        mock.lock().current = Some(text.to_owned());
        mock
    }

    /// Queues successive read results. Once the queue is empty, reads return
    /// the current text: the last text read or written.
    #[must_use]
    pub fn with_reads<'a>(self, reads: impl IntoIterator<Item = Option<&'a str>>) -> Self {
        self.lock()
            .reads
            .extend(reads.into_iter().map(|read| Ok(read.map(str::to_owned))));
        self
    }

    /// A clipboard currently holding `image` and no text.
    pub fn with_image(image: RgbaImage) -> Self {
        let mock = Self::new();
        mock.lock().current_image = Some(image);
        mock
    }

    /// Queues successive image read results. Once the queue is empty, image
    /// reads return the current image.
    #[must_use]
    pub fn with_image_reads(self, reads: impl IntoIterator<Item = Option<RgbaImage>>) -> Self {
        self.lock().image_reads.extend(reads);
        self
    }

    /// Every image written, oldest first.
    pub fn image_writes(&self) -> Vec<RgbaImage> {
        self.lock().image_writes.clone()
    }

    /// The image an image read would return once the queue is empty.
    pub fn current_image(&self) -> Option<RgbaImage> {
        self.lock().current_image.clone()
    }

    /// Makes the next queued read fail with `err`.
    pub fn push_read_error(&self, err: ClipboardError) {
        self.lock().reads.push_front(Err(err));
    }

    /// Makes every write fail (`true`) or succeed (`false`).
    pub fn fail_writes(&self, fail: bool) {
        self.lock().fail_writes = fail;
    }

    /// Every text written, oldest first.
    pub fn writes(&self) -> Vec<String> {
        self.lock().writes.clone()
    }

    /// The text a read would return once the queue is empty.
    pub fn current(&self) -> Option<String> {
        self.lock().current.clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, MockClipboardState> {
        self.0.lock().expect("mock clipboard lock")
    }
}

impl Clipboard for MockClipboard {
    fn read_text(&mut self) -> Result<Option<String>, ClipboardError> {
        let mut state = self.lock();
        match state.reads.pop_front() {
            Some(Ok(text)) => {
                if text.is_some() {
                    state.current.clone_from(&text);
                }
                Ok(text)
            }
            Some(Err(err)) => Err(err),
            None => Ok(state.current.clone()),
        }
    }

    fn write_text(&mut self, text: &str) -> Result<(), ClipboardError> {
        let mut state = self.lock();
        if state.fail_writes {
            return Err(ClipboardError::Other("injected write failure".to_owned()));
        }
        state.writes.push(text.to_owned());
        state.current = Some(text.to_owned());
        state.current_image = None;
        Ok(())
    }

    fn read_image(&mut self) -> Result<Option<RgbaImage>, ClipboardError> {
        let mut state = self.lock();
        match state.image_reads.pop_front() {
            Some(image) => {
                if image.is_some() {
                    state.current_image.clone_from(&image);
                }
                Ok(image)
            }
            None => Ok(state.current_image.clone()),
        }
    }

    fn write_image(&mut self, image: &RgbaImage) -> Result<(), ClipboardError> {
        let mut state = self.lock();
        if state.fail_writes {
            return Err(ClipboardError::Other("injected write failure".to_owned()));
        }
        state.image_writes.push(image.clone());
        state.current_image = Some(image.clone());
        state.current = None;
        Ok(())
    }
}
