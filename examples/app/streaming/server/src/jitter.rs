// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Example-only randomized delays for the demo's simulated backend work.
//!
//! Both the feed pacing and the weather endpoint want a delay that varies
//! per request, so a reload visibly re-orders how the page fills in rather
//! than replaying one hard-coded timeline. A dedicated random-number crate
//! would be a new workspace dependency earning exactly two `sleep` calls in
//! an example server, so this uses `std`'s clock plus xorshift64* — a
//! well-known three-shift generator that is far better than good enough for
//! "make the demo feel alive".
//!
//! Each [`Jitter`] is owned by a single request (the blocking render task or
//! one weather response), so there is no sharing, no lock, and no atomic on
//! the delay path.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Distinguishes seeds taken within the same clock tick. Windows' system
/// clock granularity is coarse enough that two requests can easily observe
/// the same nanosecond reading; mixing in a monotonic counter means they
/// still diverge.
static SEED_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Golden-ratio odd constant, used to scatter the counter across the whole
/// 64-bit range before it is mixed into the clock reading.
const SEED_SCATTER: u64 = 0x9E37_79B9_7F4A_7C15;

/// xorshift64*'s output multiplier.
const OUTPUT_MULTIPLIER: u64 = 0x2545_F491_4F6C_DD1D;

/// A tiny xorshift64* generator seeded from the system clock.
pub(crate) struct Jitter(u64);

impl Jitter {
    /// Seed from the wall clock, mixed with a process-wide counter.
    pub(crate) fn from_clock() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|elapsed| u64::try_from(elapsed.as_nanos()).ok())
            .unwrap_or(0);
        let counter = SEED_COUNTER.fetch_add(1, Ordering::Relaxed);
        // xorshift is degenerate at zero, so force an odd (hence non-zero)
        // state rather than branching on a value that is almost never hit.
        Self((nanos ^ counter.wrapping_mul(SEED_SCATTER)) | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(OUTPUT_MULTIPLIER)
    }

    /// A delay in `[min_ms, max_ms]`, inclusive at both ends.
    ///
    /// Returns exactly `min_ms` when the range is empty or inverted, so a
    /// caller can collapse the jitter by passing the same value twice.
    pub(crate) fn delay_ms(&mut self, min_ms: u64, max_ms: u64) -> Duration {
        if max_ms <= min_ms {
            return Duration::from_millis(min_ms);
        }
        let span = max_ms - min_ms + 1;
        Duration::from_millis(min_ms + self.next_u64() % span)
    }

    /// An index in `[0, len)`, or `0` when `len` is zero.
    pub(crate) fn index(&mut self, len: usize) -> usize {
        if len == 0 {
            return 0;
        }
        let len_u64 = u64::try_from(len).unwrap_or(1);
        usize::try_from(self.next_u64() % len_u64).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::Jitter;
    use std::time::Duration;

    #[test]
    fn delays_stay_inside_the_requested_range() {
        let mut jitter = Jitter::from_clock();
        for _ in 0..2_000 {
            let delay = jitter.delay_ms(500, 1_000);
            assert!(
                delay >= Duration::from_millis(500) && delay <= Duration::from_millis(1_000),
                "delay {delay:?} escaped [500ms, 1000ms]"
            );
        }
    }

    #[test]
    fn both_range_endpoints_are_reachable() {
        let mut jitter = Jitter::from_clock();
        let mut saw_min = false;
        let mut saw_max = false;
        // A 3-wide range: 2,000 draws makes a miss astronomically unlikely
        // without making the bound depend on a specific seed.
        for _ in 0..2_000 {
            match jitter.delay_ms(10, 12).as_millis() {
                10 => saw_min = true,
                12 => saw_max = true,
                11 => {}
                other => panic!("delay {other}ms escaped [10ms, 12ms]"),
            }
        }
        assert!(saw_min, "the inclusive lower bound was never produced");
        assert!(saw_max, "the inclusive upper bound was never produced");
    }

    #[test]
    fn an_empty_or_inverted_range_collapses_to_the_minimum() {
        let mut jitter = Jitter::from_clock();
        assert_eq!(jitter.delay_ms(300, 300), Duration::from_millis(300));
        assert_eq!(jitter.delay_ms(900, 100), Duration::from_millis(900));
    }

    #[test]
    fn indexes_stay_in_bounds_and_cover_the_whole_range() {
        let mut jitter = Jitter::from_clock();
        let mut seen = [false; 4];
        for _ in 0..2_000 {
            let index = jitter.index(seen.len());
            assert!(index < seen.len(), "index {index} escaped [0, 4)");
            seen[index] = true;
        }
        assert!(seen.iter().all(|hit| *hit), "some index was never produced");
    }

    #[test]
    fn an_empty_slice_yields_index_zero() {
        let mut jitter = Jitter::from_clock();
        assert_eq!(jitter.index(0), 0);
    }

    #[test]
    fn successive_generators_do_not_share_a_sequence() {
        // Two generators seeded back-to-back must not replay each other, or
        // every request in a burst would get identical pacing.
        let mut first = Jitter::from_clock();
        let mut second = Jitter::from_clock();
        let a: Vec<u64> = (0..8).map(|_| first.next_u64()).collect();
        let b: Vec<u64> = (0..8).map(|_| second.next_u64()).collect();
        assert_ne!(a, b, "two generators produced the same sequence");
    }

    #[test]
    fn a_single_generator_produces_varied_output() {
        let mut jitter = Jitter::from_clock();
        let first = jitter.delay_ms(0, u64::from(u32::MAX));
        let mut varied = false;
        for _ in 0..64 {
            if jitter.delay_ms(0, u64::from(u32::MAX)) != first {
                varied = true;
                break;
            }
        }
        assert!(varied, "the generator returned a constant value");
    }
}
