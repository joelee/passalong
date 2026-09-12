//! Test doubles for passalong's external interfaces.
//!
//! Compiled for this crate's own tests and, behind the `testing` feature, for
//! other crates' tests. Never enable the feature in production builds.

use std::collections::VecDeque;
use std::io;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use tracing_subscriber::fmt::MakeWriter;

use crate::clock::Clock;
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
