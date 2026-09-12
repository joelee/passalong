//! `init` installs a process-wide subscriber, so it is tested in its own
//! integration-test binary.

use passalong_core::telemetry::{self, LogLevel};
use passalong_core::testing::LogBuffer;

#[test]
fn init_installs_the_global_subscriber_once() {
    let buf = LogBuffer::default();
    telemetry::init(LogLevel::Info, buf.clone()).expect("first init succeeds");
    tracing::info!(target: "passalong_core::it", "hello");
    let out = buf.contents();
    assert!(out.ends_with(" info passalong_core::it hello\n"), "{out}");

    let again = telemetry::init(LogLevel::Info, LogBuffer::default());
    assert!(again.is_err(), "a second init must fail");
}
