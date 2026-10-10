// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Output-buffer sizing for repeated full renders of one protocol handle.

use std::sync::atomic::{AtomicUsize, Ordering};

/// Upper bound on the output reserved from earlier render sizes.
const MAX_RENDER_CAPACITY_HINT: usize = 32 * 1024 * 1024;

/// Lengths of the two most recent full renders of one protocol handle.
///
/// Hosts render the same pages repeatedly, so the next output buffer is
/// reserved up front instead of growing a large document by doubling (and
/// copying) from empty. The reservation follows the smaller of the two
/// lengths: a single large page must not make the small pages rendered after
/// it allocate (and free) a buffer sized for the large one.
///
/// Updates are relaxed and may interleave across threads. Any value is a valid
/// hint, so a lost update costs at most a reallocation.
#[derive(Default)]
pub(crate) struct RenderCapacityHint {
    last: AtomicUsize,
    previous: AtomicUsize,
}

impl RenderCapacityHint {
    /// Bytes to reserve for the next full render.
    pub(crate) fn get(&self) -> usize {
        let len = self
            .last
            .load(Ordering::Relaxed)
            .min(self.previous.load(Ordering::Relaxed));
        len.saturating_add(len / 8).min(MAX_RENDER_CAPACITY_HINT)
    }

    /// Record the length of a completed full render.
    pub(crate) fn record(&self, len: usize) {
        let last = self.last.load(Ordering::Relaxed);
        self.previous.store(last, Ordering::Relaxed);
        self.last.store(len, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_large_render_does_not_inflate_next_reservation() {
        let hint = RenderCapacityHint::default();
        hint.record(2_000);
        hint.record(2_000);
        hint.record(4_000_000);
        assert_eq!(hint.get(), 2_250);

        hint.record(4_000_000);
        assert_eq!(hint.get(), 4_500_000);
    }

    #[test]
    fn reservation_is_capped() {
        let hint = RenderCapacityHint::default();
        hint.record(usize::MAX);
        hint.record(usize::MAX);
        assert_eq!(hint.get(), MAX_RENDER_CAPACITY_HINT);
    }
}
