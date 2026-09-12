//! Logging: syslog-compatible single-line records with correlation ids.
//!
//! Every record is one line:
//!
//! ```text
//! <RFC 3339 UTC timestamp> <syslog severity> <target> [op=<id>] <message> [key=value ...]
//! ```
//!
//! Users choose one of five levels, which map to `tracing` and syslog like so:
//!
//! | Level     | `tracing` | syslog    |
//! |-----------|-----------|-----------|
//! | `error`   | `ERROR`   | `err`     |
//! | `warning` | `WARN`    | `warning` |
//! | `info`    | `INFO`    | `info`    |
//! | `verbose` | `DEBUG`   | `notice`  |
//! | `debug`   | `TRACE`   | `debug`   |
//!
//! Only the event fields in [`ALLOWED_FIELDS`] are written, so clipboard
//! text, file contents, and secrets never reach the logs even if a caller
//! records them by mistake.

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use chrono::SecondsFormat;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::filter::{LevelFilter, Targets};
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields, MakeWriter};
use tracing_subscriber::layer::{Context, SubscriberExt as _};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{Layer, Registry};

use crate::clock::{Clock, SystemClock};
use crate::random::RandomSource;

/// Event fields that may appear in log records; every other field is dropped.
pub const ALLOWED_FIELDS: &[&str] = &[
    "id", "size", "name", "path", "kind", "device", "attempt", "error", "op",
];

/// Targets that follow the selected level. Every other target belongs to a
/// third-party crate and is filtered by [`LogLevel::third_party_filter`].
const OWN_TARGETS: &[&str] = &[
    "passalong",
    "passalong_cli",
    "passalong_core",
    "passalong_ssh",
];

/// Log verbosity as exposed to users in configuration and on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogLevel {
    /// Errors only (syslog `err`).
    Error,
    /// Errors and warnings (syslog `warning`).
    Warning,
    /// Normal operational messages (syslog `info`).
    #[default]
    Info,
    /// Detailed progress (syslog `notice`).
    Verbose,
    /// Everything, including third-party debug output (syslog `debug`).
    Debug,
}

impl LogLevel {
    /// The lower-case name accepted by [`FromStr`].
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Info => "info",
            Self::Verbose => "verbose",
            Self::Debug => "debug",
        }
    }

    /// The `tracing` filter applied to passalong's own targets.
    pub fn level_filter(self) -> LevelFilter {
        match self {
            Self::Error => LevelFilter::ERROR,
            Self::Warning => LevelFilter::WARN,
            Self::Info => LevelFilter::INFO,
            Self::Verbose => LevelFilter::DEBUG,
            Self::Debug => LevelFilter::TRACE,
        }
    }

    /// The filter applied to third-party crates such as the SSH library.
    ///
    /// They are capped at warnings so `info` and `verbose` stay readable;
    /// `debug` opens them up to their own debug output for troubleshooting.
    pub fn third_party_filter(self) -> LevelFilter {
        match self {
            Self::Error => LevelFilter::ERROR,
            Self::Debug => LevelFilter::DEBUG,
            Self::Warning | Self::Info | Self::Verbose => LevelFilter::WARN,
        }
    }

    /// The syslog severity name of the most verbose records at this level.
    pub fn syslog_name(self) -> &'static str {
        match self {
            Self::Error => "err",
            Self::Warning => "warning",
            Self::Info => "info",
            Self::Verbose => "notice",
            Self::Debug => "debug",
        }
    }
}

impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned when parsing an unknown log level name.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid log level `{0}`: expected one of error, warning, info, verbose, debug")]
pub struct ParseLogLevelError(String);

impl FromStr for LogLevel {
    type Err = ParseLogLevelError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "error" => Ok(Self::Error),
            "warning" => Ok(Self::Warning),
            "info" => Ok(Self::Info),
            "verbose" => Ok(Self::Verbose),
            "debug" => Ok(Self::Debug),
            _ => Err(ParseLogLevelError(s.to_owned())),
        }
    }
}

/// Maps a `tracing` level to its syslog severity name.
pub fn syslog_name(level: Level) -> &'static str {
    match level {
        Level::ERROR => "err",
        Level::WARN => "warning",
        Level::INFO => "info",
        Level::DEBUG => "notice",
        _ => "debug",
    }
}

/// Errors from telemetry setup.
#[derive(Debug, thiserror::Error)]
pub enum TelemetryError {
    /// A process-wide subscriber is already installed.
    #[error("logging is already initialised")]
    AlreadyInitialised,
}

/// Installs the process-wide subscriber, writing records to `writer`.
///
/// The CLI passes `std::io::stderr` so user-facing output on stdout stays
/// clean.
///
/// # Errors
///
/// Returns [`TelemetryError::AlreadyInitialised`] if a global subscriber is
/// already set.
pub fn init<W>(level: LogLevel, writer: W) -> Result<(), TelemetryError>
where
    W: for<'w> MakeWriter<'w> + Send + Sync + 'static,
{
    tracing::subscriber::set_global_default(subscriber(level, Arc::new(SystemClock), writer))
        .map_err(|_| TelemetryError::AlreadyInitialised)
}

/// Builds the subscriber installed by [`init`]: level filtering, `op`
/// correlation, and [`SyslogFormat`] output to `writer`.
///
/// Exposed separately so tests can install it with
/// `tracing::subscriber::with_default` and a fixed clock.
pub fn subscriber<W>(
    level: LogLevel,
    clock: Arc<dyn Clock>,
    writer: W,
) -> impl Subscriber + Send + Sync + 'static
where
    W: for<'w> MakeWriter<'w> + Send + Sync + 'static,
{
    let targets = OWN_TARGETS.iter().fold(
        Targets::new().with_default(level.third_party_filter()),
        |targets, name| targets.with_target(*name, level.level_filter()),
    );
    Registry::default().with(targets).with(OpIdLayer).with(
        tracing_subscriber::fmt::layer()
            .event_format(SyslogFormat::new(clock))
            .with_writer(writer),
    )
}

/// Opens the span that correlates every record of one operation: a command
/// invocation or one `serve` job. Enter it for the operation's duration.
///
/// The span is created at `ERROR` level so it is enabled at every log level
/// and records at any level carry its id.
pub fn op_span(command: &str, rng: &mut dyn RandomSource) -> tracing::Span {
    tracing::error_span!("op", op = %new_op_id(rng), command = command)
}

/// Returns a new correlation id: 16 lowercase hex characters.
pub fn new_op_id(rng: &mut dyn RandomSource) -> String {
    format!("{:016x}", rng.next_u64())
}

/// Formats events as single syslog-compatible lines; see the module docs.
pub struct SyslogFormat {
    clock: Arc<dyn Clock>,
}

impl SyslogFormat {
    /// Creates a formatter that timestamps records with `clock`.
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self { clock }
    }
}

impl<S, N> FormatEvent<S, N> for SyslogFormat
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let meta = event.metadata();
        let timestamp = self.clock.now().to_rfc3339_opts(SecondsFormat::Secs, true);
        write!(
            writer,
            "{timestamp} {} {}",
            syslog_name(*meta.level()),
            meta.target()
        )?;

        // The scope iterates from the innermost span outwards, so nested
        // operations report their own id.
        if let Some(scope) = ctx.event_scope() {
            for span in scope {
                let extensions = span.extensions();
                if let Some(OpId(op)) = extensions.get::<OpId>() {
                    write!(writer, " op={op}")?;
                    break;
                }
            }
        }

        let mut fields = FieldCollector::default();
        event.record(&mut fields);
        if !fields.message.is_empty() {
            write!(writer, " {}", escape_line_breaks(&fields.message))?;
        }
        for (name, value) in &fields.pairs {
            write!(writer, " {name}={}", quote_if_needed(value))?;
        }
        writeln!(writer)
    }
}

/// Correlation id stored in the extensions of an `op` span.
struct OpId(String);

/// Copies the `op` field of new spans into their extensions so the formatter
/// can read it directly instead of re-parsing formatted span fields.
struct OpIdLayer;

impl<S> Layer<S> for OpIdLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let mut visitor = OpVisitor(None);
        attrs.record(&mut visitor);
        if let (Some(op), Some(span)) = (visitor.0, ctx.span(id)) {
            span.extensions_mut().insert(OpId(op));
        }
    }
}

struct OpVisitor(Option<String>);

impl Visit for OpVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "op" {
            self.0 = Some(format!("{value:?}"));
        }
    }
}

/// Collects the message and the allow-listed fields of one event.
#[derive(Default)]
struct FieldCollector {
    message: String,
    pairs: Vec<(&'static str, String)>,
}

impl FieldCollector {
    fn push(&mut self, field: &Field, value: String) {
        match field.name() {
            "message" => self.message = value,
            name if ALLOWED_FIELDS.contains(&name) => self.pairs.push((name, value)),
            _ => {}
        }
    }
}

impl Visit for FieldCollector {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.push(field, value.to_owned());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.push(field, format!("{value:?}"));
    }
}

/// Keeps each record on one line.
fn escape_line_breaks(message: &str) -> String {
    message.replace('\n', "\\n").replace('\r', "\\r")
}

/// Quotes a value when it would otherwise break `key=value` parsing.
fn quote_if_needed(value: &str) -> String {
    let needs_quotes = value.is_empty()
        || value
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '"' || c == '=');
    if needs_quotes {
        format!("{value:?}")
    } else {
        value.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{FixedClock, LogBuffer, SeqRandom};
    use std::str::FromStr;

    const T: &str = "2026-09-12T09:53:11Z";

    fn capture(level: LogLevel, f: impl FnOnce()) -> String {
        let buf = LogBuffer::default();
        let clock = Arc::new(FixedClock::at(T));
        let subscriber = subscriber(level, clock, buf.clone());
        tracing::subscriber::with_default(subscriber, f);
        buf.contents()
    }

    #[test]
    fn parses_the_five_level_names_case_insensitively() {
        let cases = [
            ("error", LogLevel::Error),
            ("Warning", LogLevel::Warning),
            ("INFO", LogLevel::Info),
            ("verbose", LogLevel::Verbose),
            ("debug", LogLevel::Debug),
        ];
        for (text, level) in cases {
            assert_eq!(LogLevel::from_str(text).unwrap(), level, "{text}");
            assert_eq!(level.to_string(), text.to_lowercase());
        }
    }

    #[test]
    fn rejects_unknown_level_names() {
        for text in ["warn", "trace", "notice", ""] {
            let err = LogLevel::from_str(text).unwrap_err();
            assert!(
                err.to_string()
                    .contains("error, warning, info, verbose, debug"),
                "{err}"
            );
        }
    }

    #[test]
    fn default_level_is_info() {
        assert_eq!(LogLevel::default(), LogLevel::Info);
    }

    #[test]
    fn exposed_levels_map_to_tracing_filters_and_syslog_names() {
        let cases = [
            (LogLevel::Error, LevelFilter::ERROR, Level::ERROR, "err"),
            (LogLevel::Warning, LevelFilter::WARN, Level::WARN, "warning"),
            (LogLevel::Info, LevelFilter::INFO, Level::INFO, "info"),
            (
                LogLevel::Verbose,
                LevelFilter::DEBUG,
                Level::DEBUG,
                "notice",
            ),
            (LogLevel::Debug, LevelFilter::TRACE, Level::TRACE, "debug"),
        ];
        for (exposed, filter, level, syslog) in cases {
            assert_eq!(exposed.level_filter(), filter);
            assert_eq!(syslog_name(level), syslog);
            assert_eq!(exposed.syslog_name(), syslog);
        }
    }

    #[test]
    fn renders_event_inside_op_span_in_syslog_layout() {
        let out = capture(LogLevel::Verbose, || {
            let mut rng = SeqRandom::new([0x0123_4567_89ab_cdef]);
            let span = op_span("clipboard", &mut rng);
            let _guard = span.enter();
            tracing::debug!(target: "passalong_core::x", id = "abc", size = 42_u64, "item sent");
        });
        assert_eq!(
            out,
            "2026-09-12T09:53:11Z notice passalong_core::x op=0123456789abcdef item sent id=abc size=42\n"
        );
    }

    #[test]
    fn omits_op_outside_an_op_span() {
        let out = capture(LogLevel::Info, || {
            tracing::info!(target: "passalong_core::x", "starting");
        });
        assert_eq!(
            out,
            "2026-09-12T09:53:11Z info passalong_core::x starting\n"
        );
    }

    #[test]
    fn innermost_op_span_wins() {
        let out = capture(LogLevel::Info, || {
            let mut rng = SeqRandom::new([1, 2]);
            let outer = op_span("serve", &mut rng);
            let _o = outer.enter();
            let inner = op_span("upload", &mut rng);
            let _i = inner.enter();
            tracing::info!(target: "passalong_core::x", "nested");
        });
        assert!(out.contains(" op=0000000000000002 "), "{out}");
    }

    #[test]
    fn drops_fields_outside_the_allow_list() {
        let out = capture(LogLevel::Info, || {
            tracing::info!(
                target: "passalong_core::x",
                text = "clipboard secret",
                content = "file bytes",
                passphrase = "hunter2",
                name = "a.txt",
                kind = "file",
                "sent"
            );
        });
        assert_eq!(
            out,
            "2026-09-12T09:53:11Z info passalong_core::x sent name=a.txt kind=file\n"
        );
        for secret in ["clipboard secret", "file bytes", "hunter2"] {
            assert!(!out.contains(secret));
        }
    }

    #[test]
    fn quotes_values_with_spaces_and_escapes_newlines() {
        let out = capture(LogLevel::Info, || {
            tracing::info!(target: "passalong_core::x", path = "/tmp/my file.txt", error = "", "line one\nline two");
        });
        assert_eq!(
            out,
            "2026-09-12T09:53:11Z info passalong_core::x line one\\nline two path=\"/tmp/my file.txt\" error=\"\"\n"
        );
    }

    #[test]
    fn records_display_and_error_values() {
        let err = std::io::Error::other("disk full");
        let out = capture(LogLevel::Info, || {
            tracing::warn!(target: "passalong_core::x", attempt = 3_i64, device = %"lap top", error = &err as &dyn std::error::Error, "retrying");
        });
        assert_eq!(
            out,
            "2026-09-12T09:53:11Z warning passalong_core::x retrying attempt=3 device=\"lap top\" error=\"disk full\"\n"
        );
    }

    #[test]
    fn filters_events_below_the_selected_level() {
        let out = capture(LogLevel::Warning, || {
            tracing::info!(target: "passalong_core::x", "hidden");
            tracing::error!(target: "passalong_core::x", "shown");
        });
        assert_eq!(out, "2026-09-12T09:53:11Z err passalong_core::x shown\n");
    }

    #[test]
    fn op_id_is_attached_even_at_error_level() {
        let out = capture(LogLevel::Error, || {
            let mut rng = SeqRandom::new([0xff]);
            let span = op_span("load", &mut rng);
            let _g = span.enter();
            tracing::error!(target: "passalong_core::x", "failed");
        });
        assert!(out.contains(" op=00000000000000ff failed"), "{out}");
    }

    #[test]
    fn third_party_targets_are_capped_at_warning_unless_debug() {
        let emit = || {
            tracing::info!(target: "russh::client", "chatty");
            tracing::debug!(target: "russh::client", "chattier");
            tracing::warn!(target: "russh::client", "worrying");
        };
        let verbose = capture(LogLevel::Verbose, emit);
        assert_eq!(
            verbose,
            "2026-09-12T09:53:11Z warning russh::client worrying\n"
        );
        let debug = capture(LogLevel::Debug, emit);
        assert!(debug.contains(" notice russh::client chattier"), "{debug}");
        assert!(debug.contains(" info russh::client chatty"), "{debug}");
    }

    #[test]
    fn passalong_targets_follow_the_selected_level() {
        for target_event in [
            || tracing::debug!(target: "passalong_ssh::connect", "x"),
            || tracing::debug!(target: "passalong::commands", "x"),
            || tracing::debug!(target: "passalong_cli", "x"),
        ] {
            assert_eq!(capture(LogLevel::Verbose, target_event).lines().count(), 1);
            assert_eq!(capture(LogLevel::Info, target_event).lines().count(), 0);
        }
    }

    #[test]
    fn op_ids_are_sixteen_lowercase_hex_characters() {
        let mut rng = SeqRandom::new([0xabc, u64::MAX]);
        assert_eq!(new_op_id(&mut rng), "0000000000000abc");
        assert_eq!(new_op_id(&mut rng), "ffffffffffffffff");
    }
}
