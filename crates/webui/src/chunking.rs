// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Consumer-set chunking for bundled CSS.
//!
//! Given style units that a set of CSS trees each pull in, units reached by the
//! same tree set may share one emitted file without over-delivery.
//!
//! Membership is a dense unit-major bitset of 64-bit words. Comparing two units'
//! consumer sets is then a word-slice comparison rather than a set operation,
//! which is what makes run grouping a single linear pass.
//!
//! This module is deliberately agnostic: units and consumers are opaque indices.
//! the sort that decides which units are adjacent. CSS bundling sorts by cascade
//! order so chunks stay contiguous and delivery order is preserved exactly.

use std::ops::Range;

/// A dense `unit × consumer` membership matrix.
pub(crate) struct ConsumerMatrix {
    words: usize,
    bits: Vec<u64>,
}

impl ConsumerMatrix {
    /// Allocate a zeroed matrix, or `None` when `units × consumers` is too
    /// large to address.
    pub(crate) fn new(units: usize, consumers: usize) -> Option<Self> {
        let words = consumers.div_ceil(u64::BITS as usize);
        let len = units.checked_mul(words)?;
        // The element count alone can still exceed what is addressable in
        // bytes, which `vec!` reports by aborting rather than returning.
        len.checked_mul(size_of::<u64>())?;
        Some(Self {
            words,
            bits: vec![0u64; len],
        })
    }

    /// Record that `consumer` reaches `unit`.
    pub(crate) fn insert(&mut self, unit: usize, consumer: usize) {
        let offset = unit * self.words + consumer / u64::BITS as usize;
        self.bits[offset] |= 1u64 << (consumer % u64::BITS as usize);
    }

    /// Borrow the consumer set of `unit`.
    pub(crate) fn row(&self, unit: usize) -> &[u64] {
        let start = unit * self.words;
        &self.bits[start..start + self.words]
    }
}

/// Split `units` into maximal runs whose consumer sets are identical.
///
/// Runs are index ranges into `units`, so the caller keeps whatever ordering it
/// established. Grouping only ever merges *adjacent* units, which is what lets a
/// caller with an order constraint express it purely as a sort.
pub(crate) fn group_runs(units: &[usize], matrix: &ConsumerMatrix) -> Vec<Range<usize>> {
    let mut runs: Vec<Range<usize>> = Vec::new();
    let mut start = 0usize;
    while start < units.len() {
        let row = matrix.row(units[start]);
        let mut end = start + 1;
        while end < units.len() && matrix.row(units[end]) == row {
            end += 1;
        }
        runs.push(start..end);
        start = end;
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matrix(units: usize, consumers: usize, edges: &[(usize, usize)]) -> ConsumerMatrix {
        let mut matrix = ConsumerMatrix::new(units, consumers).expect("matrix");
        for (unit, consumer) in edges {
            matrix.insert(*unit, *consumer);
        }
        matrix
    }

    #[test]
    fn groups_only_adjacent_units_with_equal_consumer_sets() {
        // Units 0 and 2 share a consumer set but are separated by unit 1, so
        // the caller-established order keeps them in distinct runs.
        let matrix = matrix(3, 2, &[(0, 0), (0, 1), (1, 0), (2, 0), (2, 1)]);

        assert_eq!(group_runs(&[0, 1, 2], &matrix), [0..1, 1..2, 2..3]);
        assert_eq!(group_runs(&[0, 2, 1], &matrix), [0..2, 2..3]);
    }

    #[test]
    fn groups_an_empty_slice_into_no_runs() {
        let matrix = matrix(1, 1, &[]);

        assert!(group_runs(&[], &matrix).is_empty());
    }

    #[test]
    fn rejects_a_matrix_that_cannot_be_indexed() {
        assert!(ConsumerMatrix::new(usize::MAX, 64).is_none());
    }
}
