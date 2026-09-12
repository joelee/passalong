//! Capped exponential backoff for retrying uploads.

use std::time::Duration;

const FIRST_DELAY: Duration = Duration::from_secs(1);
const MAX_DELAY: Duration = Duration::from_secs(60);

/// Delays of 1, 2, 4 … seconds, capped at 60.
#[derive(Debug, Clone)]
pub struct Backoff {
    next: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self { next: FIRST_DELAY }
    }
}

impl Backoff {
    /// The delay before the next attempt.
    pub fn next_delay(&mut self) -> Duration {
        let delay = self.next;
        self.next = (delay * 2).min(MAX_DELAY);
        delay
    }

    /// Starts again from one second.
    pub fn reset(&mut self) {
        self.next = FIRST_DELAY;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn backoff_doubles_from_one_second_up_to_a_minute() {
        let mut backoff = Backoff::default();
        let secs: Vec<u64> = (0..8).map(|_| backoff.next_delay().as_secs()).collect();
        assert_eq!(secs, [1, 2, 4, 8, 16, 32, 60, 60]);
    }

    #[test]
    fn backoff_starts_over_after_a_reset() {
        let mut backoff = Backoff::default();
        backoff.next_delay();
        backoff.next_delay();
        backoff.reset();
        assert_eq!(backoff.next_delay(), Duration::from_secs(1));
    }
}
