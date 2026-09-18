// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::HashSet;
use std::hash::Hash;

#[derive(Default, Hash, PartialEq, Eq)]
struct ScopedVisit<'a, K> {
    node: K,
    names: Vec<&'a str>,
}

// Shared graph analyses depend on name membership, not frame order, shadow
// count, or callsite identity. Never deduplicate genuinely different name sets.
#[derive(Default)]
pub(crate) struct ScopedVisits<'a, K> {
    seen: HashSet<ScopedVisit<'a, K>>,
    scratch: ScopedVisit<'a, K>,
}

impl<'a, K: Default + Eq + Hash> ScopedVisits<'a, K> {
    pub(crate) fn insert(&mut self, node: K, names: impl Iterator<Item = &'a str>) -> bool {
        self.scratch.node = node;
        self.scratch.names.clear();
        self.scratch.names.extend(names);
        self.scratch.names.sort_unstable();
        self.scratch.names.dedup();
        if self.seen.contains(&self.scratch) {
            return false;
        }
        self.seen.insert(std::mem::take(&mut self.scratch));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::ScopedVisits;

    #[test]
    fn visits_are_keyed_by_node_and_canonical_name_set() {
        let mut visits = ScopedVisits::default();
        assert!(visits.insert(1, ["a", "b"].into_iter()));
        assert!(!visits.insert(1, ["b", "a", "a"].into_iter()));
        assert!(visits.insert(1, ["b", "c"].into_iter()));
        assert!(visits.insert(1, ["a"].into_iter()));
        assert!(visits.insert(2, ["a", "b"].into_iter()));
        assert!(visits.insert(1, [].into_iter()));
        assert!(!visits.insert(1, [].into_iter()));
    }
}
