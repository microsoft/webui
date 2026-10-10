// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Per-route output capacity hints shared by the in-process host bindings.

use std::sync::atomic::{AtomicUsize, Ordering};

const INITIAL_RENDER_CAPACITY: usize = 4 * 1024;
const MAX_RENDER_CAPACITY_HINT: usize = 1024 * 1024;
const RENDER_CAPACITY_BUCKETS: usize = 64;
const RENDER_CAPACITY_MASK: usize = (1 << 21) - 1;
const _: () = assert!(RENDER_CAPACITY_BUCKETS.is_power_of_two());
const _: () = assert!(MAX_RENDER_CAPACITY_HINT <= RENDER_CAPACITY_MASK);

#[cfg(target_pointer_width = "64")]
const CAPACITY_HASH_OFFSET: usize = 14_695_981_039_346_656_037;
#[cfg(target_pointer_width = "64")]
const CAPACITY_HASH_PRIME: usize = 1_099_511_628_211;
#[cfg(target_pointer_width = "32")]
const CAPACITY_HASH_OFFSET: usize = 2_166_136_261;
#[cfg(target_pointer_width = "32")]
const CAPACITY_HASH_PRIME: usize = 16_777_619;

/// Remembers recent full-render output sizes per entry and request path.
///
/// Bindings that buffer a whole document use the hint as the initial output
/// capacity, so repeated renders of a route avoid incremental buffer growth.
/// Hints are advisory: a miss returns a small default, and a stale hint only
/// changes allocation size, never the rendered bytes.
///
/// A fixed-size direct-mapped cache keeps memory bounded. Each atomic packs the
/// capacity with a fingerprint, so bucket collisions become misses rather than
/// cross-route over-allocation. Races affect only an advisory allocation size.
pub struct RenderCapacityHints {
    buckets: [AtomicUsize; RENDER_CAPACITY_BUCKETS],
}

impl RenderCapacityHints {
    /// Create an empty hint cache.
    #[must_use]
    pub fn new() -> Self {
        Self {
            buckets: std::array::from_fn(|_| AtomicUsize::new(0)),
        }
    }

    /// Return the initial output capacity for a route's next full render.
    #[must_use]
    pub fn load(&self, entry_id: &str, request_path: &str) -> usize {
        let hash = capacity_hint_hash(entry_id, request_path);
        let encoded = self.buckets[hash & (RENDER_CAPACITY_BUCKETS - 1)].load(Ordering::Relaxed);
        if encoded & !RENDER_CAPACITY_MASK == capacity_hint_fingerprint(hash) {
            encoded & RENDER_CAPACITY_MASK
        } else {
            INITIAL_RENDER_CAPACITY
        }
    }

    /// Record a completed render's output length, clamped to a bounded range.
    pub fn store(&self, entry_id: &str, request_path: &str, capacity: usize) {
        let hash = capacity_hint_hash(entry_id, request_path);
        let encoded = capacity_hint_fingerprint(hash)
            | capacity.clamp(INITIAL_RENDER_CAPACITY, MAX_RENDER_CAPACITY_HINT);
        self.buckets[hash & (RENDER_CAPACITY_BUCKETS - 1)].store(encoded, Ordering::Relaxed);
    }
}

impl Default for RenderCapacityHints {
    fn default() -> Self {
        Self::new()
    }
}

fn capacity_hint_hash(entry_id: &str, request_path: &str) -> usize {
    let mut hash = CAPACITY_HASH_OFFSET;
    for byte in entry_id
        .bytes()
        .chain(std::iter::once(u8::MAX))
        .chain(request_path.bytes())
    {
        hash ^= usize::from(byte);
        hash = hash.wrapping_mul(CAPACITY_HASH_PRIME);
    }
    hash
}

fn capacity_hint_fingerprint(hash: usize) -> usize {
    let fingerprint = hash.rotate_left(7) & !RENDER_CAPACITY_MASK;
    if fingerprint == 0 {
        RENDER_CAPACITY_MASK + 1
    } else {
        fingerprint
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_capacity_hints_isolate_entries_and_routes() {
        let hints = RenderCapacityHints::new();

        hints.store("index.html", "/contacts", MAX_RENDER_CAPACITY_HINT);

        assert_eq!(
            hints.load("index.html", "/"),
            INITIAL_RENDER_CAPACITY,
            "a small route must not inherit a large route's capacity"
        );
        assert_eq!(
            hints.load("contacts.html", "/contacts"),
            INITIAL_RENDER_CAPACITY,
            "another entry must not inherit the large entry's capacity"
        );
        assert_eq!(
            hints.load("index.html", "/contacts"),
            MAX_RENDER_CAPACITY_HINT
        );
    }

    #[test]
    fn render_capacity_hints_do_not_leak_across_bucket_collisions() {
        let entry_id = "index.html";
        let large_path = "/contacts";
        let large_hash = capacity_hint_hash(entry_id, large_path);
        let colliding_path = (0..RENDER_CAPACITY_BUCKETS * 16)
            .map(|index| format!("/route-{index}"))
            .find(|path| {
                let hash = capacity_hint_hash(entry_id, path);
                hash & (RENDER_CAPACITY_BUCKETS - 1) == large_hash & (RENDER_CAPACITY_BUCKETS - 1)
                    && capacity_hint_fingerprint(hash) != capacity_hint_fingerprint(large_hash)
            })
            .unwrap_or_else(|| panic!("a same-bucket route should be found"));
        let hints = RenderCapacityHints::new();

        hints.store(entry_id, large_path, MAX_RENDER_CAPACITY_HINT);
        assert_eq!(
            hints.load(entry_id, &colliding_path),
            INITIAL_RENDER_CAPACITY
        );

        hints.store(entry_id, &colliding_path, INITIAL_RENDER_CAPACITY * 2);
        assert_eq!(
            hints.load(entry_id, large_path),
            INITIAL_RENDER_CAPACITY,
            "an overwritten bucket must miss rather than return another route's hint"
        );
        assert_eq!(
            hints.load(entry_id, &colliding_path),
            INITIAL_RENDER_CAPACITY * 2
        );
    }

    #[test]
    fn render_capacity_hints_clamp_retained_sizes() {
        let hints = RenderCapacityHints::new();

        hints.store("index.html", "/", 0);
        assert_eq!(hints.load("index.html", "/"), INITIAL_RENDER_CAPACITY);

        hints.store("index.html", "/", usize::MAX);
        assert_eq!(hints.load("index.html", "/"), MAX_RENDER_CAPACITY_HINT);
    }
}
