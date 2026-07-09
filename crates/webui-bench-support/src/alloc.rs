// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Allocation-counting global allocator — the **memory** dimension.
//!
//! Wrap the system allocator so every allocation is counted (count + bytes).
//! Unlike a host-process working-set probe, this is *exact* and *deterministic*:
//! it observes the transient owned trees a render builds and frees within a
//! single call, which never surface in `process.memoryUsage()` on the Node side.
//!
//! # Usage
//!
//! ```ignore
//! use webui_bench_support::alloc::CountingAllocator;
//!
//! #[global_allocator]
//! static GLOBAL: CountingAllocator = CountingAllocator::new();
//! ```
//!
//! Then read [`snapshot`] before and after a workload; the difference is the
//! allocation the workload performed.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);
static ALLOC_BYTES: AtomicUsize = AtomicUsize::new(0);

/// A [`GlobalAlloc`] that forwards to the system allocator while counting the
/// number of allocations and the total bytes requested.
///
/// Install exactly one as the process `#[global_allocator]`. Reallocations that
/// grow a block are counted as one additional allocation of the growth delta,
/// matching how an owned tree expands its backing buffers.
pub struct CountingAllocator;

impl CountingAllocator {
    /// Construct the allocator. `const` so it can initialise a `static`.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for CountingAllocator {
    fn default() -> Self {
        Self::new()
    }
}

// SAFETY: every method forwards to `System` with the exact pointer/layout the
// caller provided; the atomic counters have no bearing on allocation validity.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        // SAFETY: forwarded with the same layout the caller produced.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwarded; ptr/layout came from `alloc` above.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        // SAFETY: forwarded with the same layout the caller produced.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if new_size > layout.size() {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
            ALLOC_BYTES.fetch_add(new_size - layout.size(), Ordering::Relaxed);
        }
        // SAFETY: forwarded; ptr/layout came from `alloc` above.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

/// A point-in-time reading of the process-wide allocation counters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AllocStats {
    /// Total number of allocations (including growing reallocations) so far.
    pub count: usize,
    /// Total bytes requested so far.
    pub bytes: usize,
}

impl AllocStats {
    /// Saturating difference `self - earlier`, i.e. the allocation performed
    /// between two [`snapshot`] readings.
    #[must_use]
    pub fn since(self, earlier: AllocStats) -> AllocStats {
        AllocStats {
            count: self.count.saturating_sub(earlier.count),
            bytes: self.bytes.saturating_sub(earlier.bytes),
        }
    }
}

/// Read the current process-wide allocation counters.
///
/// Meaningful only when [`CountingAllocator`] is installed as the
/// `#[global_allocator]`; otherwise the counters stay at zero.
#[must_use]
pub fn snapshot() -> AllocStats {
    AllocStats {
        count: ALLOC_COUNT.load(Ordering::Relaxed),
        bytes: ALLOC_BYTES.load(Ordering::Relaxed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn since_is_saturating() {
        let base = AllocStats {
            count: 10,
            bytes: 1000,
        };
        let later = AllocStats {
            count: 15,
            bytes: 1500,
        };
        assert_eq!(
            later.since(base),
            AllocStats {
                count: 5,
                bytes: 500
            }
        );
        // Reversed order must not underflow.
        assert_eq!(base.since(later), AllocStats { count: 0, bytes: 0 });
    }

    #[test]
    fn snapshot_is_monotonic() {
        // The counters are process-global and only ever increase; two reads
        // must be ordered even without our allocator installed in the test bin.
        let a = snapshot();
        let b = snapshot();
        assert!(b.count >= a.count);
        assert!(b.bytes >= a.bytes);
    }
}
