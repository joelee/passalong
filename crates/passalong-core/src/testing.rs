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
}

#[derive(Debug, Default)]
struct FaultState {
    calls: HashMap<FsOp, usize>,
    failing: HashSet<(FsOp, usize)>,
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

    fn check(&self, op: FsOp, path: &RemotePath) -> Result<(), FsError> {
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
        Ok(())
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
        self.check(FsOp::OpenWrite, path)?;
        self.inner.open_write(path).await
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
