//! Time source abstraction, so time-dependent code is deterministic in tests.

use chrono::{DateTime, Utc};

/// Source of the current time.
pub trait Clock: Send + Sync {
    /// Returns the current instant in UTC.
    fn now(&self) -> DateTime<Utc>;
}

/// [`Clock`] backed by the system clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_reports_current_time() {
        let before = Utc::now();
        let now = SystemClock.now();
        assert!(now >= before && now <= Utc::now());
    }

    #[test]
    fn fixed_clock_returns_the_configured_instant() {
        let clock = crate::testing::FixedClock::at("2026-09-12T09:53:11Z");
        assert_eq!(clock.now().timestamp(), 1_789_206_791);
        assert_eq!(clock.now(), clock.now());
    }
}
