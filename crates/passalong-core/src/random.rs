//! Non-cryptographic randomness for correlation ids and temporary names.
//!
//! Nothing here is suitable for secrets. Code that needs values unlikely to
//! repeat takes an injectable [`RandomSource`] so tests stay deterministic,
//! and the default source needs no extra dependency.

use std::hash::{BuildHasher, Hasher, RandomState};

/// Source of random `u64` values.
pub trait RandomSource: Send {
    /// Returns the next value.
    fn next_u64(&mut self) -> u64;
}

/// [`RandomSource`] keyed by the standard library's per-process random
/// SipHash keys.
///
/// Each value is the keyed hash of an incrementing counter, so a generator
/// does not repeat itself in practice and two generators produce different
/// sequences because every [`RandomState`] gets fresh keys.
#[derive(Debug, Clone, Default)]
pub struct StdRandom {
    state: RandomState,
    counter: u64,
}

impl StdRandom {
    /// Creates a generator with fresh random keys.
    pub fn new() -> Self {
        Self::default()
    }
}

impl RandomSource for StdRandom {
    fn next_u64(&mut self) -> u64 {
        self.counter = self.counter.wrapping_add(1);
        let mut hasher = self.state.build_hasher();
        hasher.write_u64(self.counter);
        hasher.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn std_random_yields_distinct_values() {
        let mut rng = StdRandom::new();
        let values: std::collections::HashSet<u64> = (0..1000).map(|_| rng.next_u64()).collect();
        assert_eq!(values.len(), 1000);
    }

    #[test]
    fn independent_generators_differ() {
        assert_ne!(StdRandom::new().next_u64(), StdRandom::new().next_u64());
    }
}
