// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Build-time interning of state paths.
//!
//! Every dotted path a template references is discovered while the protocol is
//! assembled, stored once in a [`PathTable`], and replaced at each use site by a
//! small integer id. Rendering then never scans, splits, or hashes a path
//! string.
//!
//! Two distinct wins come out of this, and they are worth separating:
//!
//! * **Segmentation.** Splitting `"user.profile.name"` on `.` is pure repeated
//!   work: the answer is identical on every render. [`PathEntry::segment_ends`]
//!   records the split once so a handler slices the segments zero-copy.
//! * **Slots.** Most paths resolve straight to global state. Those are given a
//!   dense slot index, so a handler can resolve them once per render into a flat
//!   table and index it afterwards instead of walking the state map per use.
//!
//! # Why some paths do not get a slot
//!
//! Path resolution is hierarchical: a loop item (`<for each="item in items">`)
//! or a component prop shadows global state for the *first* segment of a path.
//! A slot is only sound when that shadowing can never apply, because the slot
//! table is filled from global state alone.
//!
//! Rather than prove per-scope reachability, this pass uses a whole-program
//! rule that is conservative by construction:
//!
//! > A path is interning-eligible when its first segment is not used as a loop
//! > item name or an attribute name *anywhere* in the protocol.
//!
//! If a name is never bound as a loop item or a prop, no scope can bind it, so
//! resolving it through the hierarchy is always equivalent to reading global
//! state. Names that do collide simply keep the existing string-resolution
//! path, which is correct, just not accelerated. This trades a small number of
//! interned paths for a rule that needs no fixpoint over the fragment graph and
//! cannot silently go wrong as scoping rules evolve.
//!
//! Path ids are assigned in a deterministic order (fragment records visited by
//! sorted id) so repeated builds of the same input produce byte-identical
//! protocols.

use std::collections::{HashMap, HashSet};

use crate::{
    condition_expr, web_ui_fragment::Fragment, ConditionExpr, PathEntry, PathTable, Predicate,
    WebUIFragmentRecords,
};

/// Assign path ids to every path-bearing fragment field and return the table
/// describing them.
///
/// Runs once per build. Fragments whose paths cannot be interned keep id `0`,
/// which handlers treat as "resolve this the ordinary way".
pub(crate) fn build(records: &mut WebUIFragmentRecords) -> PathTable {
    let shadowed = collect_shadowed_names(records);
    let mut interner = Interner::new(shadowed);

    // Visit records in sorted id order so path ids - and therefore the emitted
    // bytes - are reproducible across builds.
    let mut record_ids: Vec<String> = records.keys().cloned().collect();
    record_ids.sort_unstable();

    for record_id in &record_ids {
        let Some(list) = records.get_mut(record_id) else {
            continue;
        };
        for fragment in &mut list.fragments {
            let Some(fragment) = fragment.fragment.as_mut() else {
                continue;
            };
            match fragment {
                Fragment::Signal(signal) => {
                    // Structural signals are intercepted by the handler before
                    // any state lookup, so an id here would only add a table
                    // entry and wire bytes that can never be read.
                    if !signal.is_structural() {
                        signal.path_id = interner.intern(&signal.value);
                    }
                }
                Fragment::Attribute(attribute) => {
                    if !attribute.raw_value {
                        attribute.path_id = interner.intern(&attribute.value);
                    }
                    if let Some(condition) = attribute.condition_tree.as_mut() {
                        intern_condition(condition, &mut interner);
                    }
                }
                Fragment::IfCond(if_cond) => {
                    if let Some(condition) = if_cond.condition.as_mut() {
                        intern_condition(condition, &mut interner);
                    }
                }
                _ => {}
            }
        }
    }

    interner.finish()
}

/// Collect every name that can shadow the first segment of a path.
///
/// These are exactly the two bindings the handler consults before falling back
/// to global state: loop item names, and attribute names (which become props
/// inside a component body). Attribute names are taken wholesale rather than
/// narrowed to component call sites - over-collecting only costs a slot.
fn collect_shadowed_names(records: &WebUIFragmentRecords) -> HashSet<String> {
    let mut shadowed = HashSet::new();
    for list in records.values() {
        for fragment in &list.fragments {
            match fragment.fragment.as_ref() {
                Some(Fragment::ForLoop(for_loop)) => {
                    shadowed.insert(for_loop.item.clone());
                }
                Some(Fragment::Attribute(attribute)) => {
                    // A component prop is bound under its camelCase form, so
                    // `data-title` shadows the path `dataTitle`. Record every
                    // spelling an attribute can take; an extra name only costs
                    // a slot, while a missing one would be a correctness bug.
                    // A `:` prefix marks a complex binding; the prop is bound
                    // under the bare name.
                    let name = attribute.name.strip_prefix(':').unwrap_or(&attribute.name);
                    shadowed.insert(name.to_string());
                    shadowed.insert(crate::attrs::attribute_to_camel(name));
                    shadowed.insert(crate::attrs::camel_to_kebab(name));
                }
                _ => {}
            }
        }
    }
    shadowed
}

/// Walk a condition tree iteratively, interning each operand that is a path.
///
/// The traversal is explicit rather than recursive so deeply nested conditions
/// cannot overflow the stack at build time.
/// The traversal buffer is allocated per call rather than reused across
/// fragments: a reused buffer would tie every fragment's mutable borrow to one
/// lifetime. This is build-time work on small trees, so the allocation is not
/// on any measured path.
fn intern_condition(root: &mut ConditionExpr, interner: &mut Interner) {
    let mut stack: Vec<&mut ConditionExpr> = Vec::new();
    stack.push(root);
    while let Some(node) = stack.pop() {
        match node.expr.as_mut() {
            Some(condition_expr::Expr::Identifier(identifier)) => {
                identifier.path_id = interner.intern(&identifier.value);
            }
            Some(condition_expr::Expr::Predicate(predicate)) => {
                predicate.left_path_id = interner.intern(&predicate.left);
                // The right operand is a path only when it is not a literal;
                // `is_literal_operand` is the same test the evaluator uses.
                if !Predicate::is_literal_operand(&predicate.right) {
                    predicate.right_path_id = interner.intern(&predicate.right);
                }
            }
            Some(condition_expr::Expr::Not(not)) => {
                if let Some(inner) = not.condition.as_mut() {
                    stack.push(inner);
                }
            }
            Some(condition_expr::Expr::Compound(compound)) => {
                if let Some(left) = compound.left.as_mut() {
                    stack.push(left);
                }
                if let Some(right) = compound.right.as_mut() {
                    stack.push(right);
                }
            }
            None => {}
        }
    }
}

/// Assigns 1-based ids to distinct directly-resolvable paths, in first-seen
/// order.
///
/// A path that any scope could shadow, or that contains a synthetic `length`
/// segment, is never given an id. Interning it would only make handlers load a
/// table entry to discover the path is unusable, so `0` carries that answer
/// with no memory traffic at all.
struct Interner {
    /// Owns every interned path string exactly once.
    ids: HashMap<String, u32>,
    /// Segment boundaries for each interned path, indexed by `id - 1`.
    segments: Vec<Vec<u32>>,
    /// First segments that a loop item or component prop binds somewhere in
    /// this protocol.
    shadowed: HashSet<String>,
}

impl Interner {
    fn new(shadowed: HashSet<String>) -> Self {
        Self {
            ids: HashMap::new(),
            segments: Vec::new(),
            shadowed,
        }
    }

    /// Return the 1-based id for `path`, assigning one if it is new.
    ///
    /// Returns `0` when `path` is empty or is not directly resolvable, which is
    /// the encoding for "resolve this by name through the scope hierarchy".
    fn intern(&mut self, path: &str) -> u32 {
        if path.is_empty() {
            return 0;
        }
        if let Some(&id) = self.ids.get(path) {
            return id;
        }
        let segment_ends = segment_ends(path);
        // The first segment is the only one a scope can bind, so it alone
        // decides whether the rest of the path is reachable directly.
        let first_segment_end = segment_ends.first().copied().unwrap_or(0) as usize;
        let Some(first_segment) = path.get(..first_segment_end) else {
            return 0;
        };
        if self.shadowed.contains(first_segment) || has_length_segment(path, &segment_ends) {
            return 0;
        }
        // Ids are 1-based so that `0` stays available as the absent marker and
        // stays off the wire under proto3 default-value elision.
        let id = u32::try_from(self.segments.len() + 1).unwrap_or(0);
        if id == 0 {
            return 0;
        }
        // The path is allocated once here; `finish` moves that same string
        // into the emitted entry rather than cloning it.
        self.ids.insert(path.to_owned(), id);
        self.segments.push(segment_ends);
        id
    }

    fn finish(mut self) -> PathTable {
        let mut paths: Vec<PathEntry> = self
            .segments
            .drain(..)
            .map(|segment_ends| PathEntry {
                path: String::new(),
                segment_ends,
            })
            .collect();
        // Move each owned path out of the index and into its id-ordered slot.
        for (path, id) in self.ids {
            let Some(slot) = id.checked_sub(1).and_then(|i| paths.get_mut(i as usize)) else {
                continue;
            };
            slot.path = path;
        }
        PathTable { paths }
    }
}

/// Whether any segment of `path` is `length`.
///
/// `length` is synthetic on arrays and strings: resolving it produces an owned
/// value that does not exist in the state tree, and on an array it short-
/// circuits the rest of the path. Slots hold plain borrows into state and
/// resolve segments uniformly, so paths that can hit that rule are excluded
/// from slot assignment and keep the general resolution path.
fn has_length_segment(path: &str, segment_ends: &[u32]) -> bool {
    let mut start = 0usize;
    for &end in segment_ends {
        if path.get(start..end as usize) == Some("length") {
            return true;
        }
        start = end as usize + 1;
    }
    false
}

/// Record the end offset of every dot-separated segment of `path`.
///
/// `"a.bb.c"` yields `[1, 4, 6]`; a path with no dots yields a single entry
/// equal to its length. Offsets are byte indices, so slicing with them is safe
/// for any UTF-8 path because `.` is ASCII and never splits a code point.
fn segment_ends(path: &str) -> Vec<u32> {
    let mut ends = Vec::new();
    for (index, byte) in path.bytes().enumerate() {
        if byte == b'.' {
            // `index` fits in u32 for any path a template can express.
            if let Ok(end) = u32::try_from(index) {
                ends.push(end);
            }
        }
    }
    if let Ok(end) = u32::try_from(path.len()) {
        ends.push(end);
    }
    ends
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FragmentList, WebUIFragment};

    fn records(entries: Vec<(&str, Vec<WebUIFragment>)>) -> WebUIFragmentRecords {
        entries
            .into_iter()
            .map(|(id, fragments)| {
                (
                    id.to_string(),
                    FragmentList {
                        fragments,
                        contains_boundary: false,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn interns_signal_paths_and_shares_ids() {
        let mut recs = records(vec![(
            "index.html",
            vec![
                WebUIFragment::signal("user.name", false),
                WebUIFragment::signal("user.name", false),
                WebUIFragment::signal("title", false),
            ],
        )]);
        let table = build(&mut recs);

        assert_eq!(table.paths.len(), 2, "identical paths share one entry");
        let fragments = &recs["index.html"].fragments;
        let ids: Vec<u32> = fragments
            .iter()
            .map(|f| match f.fragment.as_ref() {
                Some(Fragment::Signal(signal)) => signal.path_id,
                _ => 0,
            })
            .collect();
        assert_eq!(ids, vec![1, 1, 2], "repeat path reuses its id");
    }

    #[test]
    fn segment_ends_describe_the_split() {
        assert_eq!(segment_ends("a.bb.c"), vec![1, 4, 6]);
        assert_eq!(segment_ends("solo"), vec![4]);
        let entry = &build(&mut records(vec![(
            "x",
            vec![WebUIFragment::signal("user.profile.name", false)],
        )]))
        .paths[0];
        let ends = &entry.segment_ends;
        assert_eq!(&entry.path[..ends[0] as usize], "user");
        assert_eq!(
            &entry.path[ends[0] as usize + 1..ends[1] as usize],
            "profile"
        );
    }

    #[test]
    fn structural_signals_are_not_interned() {
        // Compiler-owned signals are intercepted before any state lookup, so
        // interning them would only bloat the table and the emitted bytes.
        let mut recs = records(vec![(
            "index.html",
            vec![
                WebUIFragment::signal("}}}webui:head_start", true),
                WebUIFragment::signal("}}}webui:shadow_styles:greeting-card", true),
                WebUIFragment::signal("title", false),
            ],
        )]);
        let table = build(&mut recs);

        assert_eq!(
            table
                .paths
                .iter()
                .map(|e| e.path.as_str())
                .collect::<Vec<_>>(),
            vec!["title"],
            "only the real state path is interned"
        );

        let fragments = &recs["index.html"].fragments;
        for (index, fragment) in fragments.iter().enumerate().take(2) {
            match fragment.fragment.as_ref() {
                Some(Fragment::Signal(signal)) => {
                    assert_eq!(signal.path_id, 0, "structural signal {index} kept id 0");
                }
                _ => unreachable!("fragment is a signal"),
            }
        }
    }

    #[test]
    fn loop_item_name_blocks_interning() {
        // `item` is bound by the loop, so `item.name` must keep resolving
        // through the scope hierarchy rather than reading global state.
        let mut recs = records(vec![
            (
                "index.html",
                vec![WebUIFragment::for_loop("item", "rows", "for_1")],
            ),
            ("for_1", vec![WebUIFragment::signal("item.name", false)]),
        ]);
        let table = build(&mut recs);

        assert!(
            !table.paths.iter().any(|entry| entry.path == "item.name"),
            "a shadowed path is never interned"
        );
        let signal = match recs["for_1"].fragments[0].fragment.as_ref() {
            Some(Fragment::Signal(signal)) => signal,
            _ => unreachable!("fragment is a signal"),
        };
        assert_eq!(signal.path_id, 0, "shadowed signal keeps the string path");
    }

    #[test]
    fn attribute_name_blocks_interning() {
        // An attribute name becomes a component prop, which shadows the first
        // segment of any path inside that component.
        let mut recs = records(vec![(
            "index.html",
            vec![
                WebUIFragment::attribute("title", "pageTitle"),
                WebUIFragment::signal("title.text", false),
            ],
        )]);
        let table = build(&mut recs);

        assert!(
            !table.paths.iter().any(|entry| entry.path == "title.text"),
            "a component prop shadows the first segment"
        );
    }

    #[test]
    fn predicate_literals_are_not_interned_as_paths() {
        let condition =
            ConditionExpr::predicate("count", crate::ComparisonOperator::GreaterThan, "5");
        let mut recs = records(vec![(
            "index.html",
            vec![WebUIFragment::if_cond(condition, "if_1")],
        )]);
        let table = build(&mut recs);

        assert_eq!(table.paths.len(), 1, "only the left operand is a path");
        assert_eq!(table.paths[0].path, "count");
    }

    #[test]
    fn predicate_path_operands_are_both_interned() {
        let condition =
            ConditionExpr::predicate("count", crate::ComparisonOperator::Equal, "limit");
        let mut recs = records(vec![(
            "index.html",
            vec![WebUIFragment::if_cond(condition, "if_1")],
        )]);
        let table = build(&mut recs);

        let names: Vec<&str> = table.paths.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(names, vec!["count", "limit"]);
    }

    #[test]
    fn deeply_nested_conditions_do_not_recurse() {
        // Build-time interning must survive nesting far deeper than a recursive
        // walk would tolerate.
        let mut condition = ConditionExpr::identifier("leaf");
        for _ in 0..2_000 {
            condition = ConditionExpr::negated(condition);
        }
        let mut recs = records(vec![(
            "index.html",
            vec![WebUIFragment::if_cond(condition, "if_1")],
        )]);
        let table = build(&mut recs);
        assert_eq!(table.paths.len(), 1);
        assert_eq!(table.paths[0].path, "leaf");
    }

    #[test]
    fn length_paths_are_never_interned() {
        // `items.length` is synthetic, so it must keep resolving through the
        // general path rather than being read straight out of state.
        let mut recs = records(vec![(
            "index.html",
            vec![
                WebUIFragment::signal("items.length", false),
                WebUIFragment::signal("items", false),
            ],
        )]);
        let table = build(&mut recs);

        assert!(
            !table.paths.iter().any(|entry| entry.path == "items.length"),
            "a synthetic length path is never interned"
        );
        assert!(
            table.paths.iter().any(|entry| entry.path == "items"),
            "the array itself is still interned"
        );
    }

    #[test]
    fn ids_are_dense_and_skip_uninternable_paths() {
        let mut recs = records(vec![(
            "index.html",
            vec![
                WebUIFragment::signal("a", false),
                WebUIFragment::signal("b.length", false),
                WebUIFragment::signal("c", false),
            ],
        )]);
        let table = build(&mut recs);
        let names: Vec<&str> = table.paths.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(names, vec!["a", "c"], "a skipped path takes no id");
        let ids: Vec<u32> = recs["index.html"]
            .fragments
            .iter()
            .map(|f| match f.fragment.as_ref() {
                Some(Fragment::Signal(signal)) => signal.path_id,
                _ => unreachable!("fragment is a signal"),
            })
            .collect();
        assert_eq!(ids, vec![1, 0, 2]);
    }

    #[test]
    fn path_ids_are_deterministic_across_builds() {
        let make = || {
            records(vec![
                ("b.html", vec![WebUIFragment::signal("beta", false)]),
                ("a.html", vec![WebUIFragment::signal("alpha", false)]),
            ])
        };
        let first = build(&mut make());
        let second = build(&mut make());
        let names = |table: &PathTable| -> Vec<String> {
            table.paths.iter().map(|e| e.path.clone()).collect()
        };
        assert_eq!(names(&first), names(&second));
        assert_eq!(names(&first), vec!["alpha".to_string(), "beta".to_string()]);
    }
}
