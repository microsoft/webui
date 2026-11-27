// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

mod owned;
mod shape;

pub(crate) use owned::project_owned_paths;

use std::collections::{BTreeMap, HashSet};

use serde_json::{Map, Value};
use webui_protocol::{HydrationReadField, HydrationReadPlan, HydrationReadScope, WebUIProtocol};

use crate::{HandlerError, KeyView, Result, STATE_INJECT_KEY};

pub(crate) fn compact_string_length(byte_len: usize) -> bool {
    // A small string can cost less than a one-field object in both a parked
    // serde_json snapshot and the browser heap. Don't trade one for the other.
    byte_len >= 1024
}

#[derive(Clone, Copy)]
struct Selector<'a> {
    plan: &'a HydrationReadPlan,
    node: u32,
}

pub(crate) struct ReadSelection<'a> {
    roots: BTreeMap<&'a str, Vec<Selector<'a>>>,
    active_lazy: Option<HashSet<&'a str>>,
}

// A continuation doesn't need a second copy of the lexical repeat graph.
// Collections, conditions and other SSR inputs already retain whole roots.
// Only independent event arguments add field paths; scope edges (including
// closed lazy regions) keep the same input and never expand a repeat callsite.
pub(crate) struct FrozenEventRoot {
    pub(crate) key: Box<str>,
    paths: Box<[Box<[HydrationReadField]>]>,
    compact_lengths: bool,
}

fn event_paths<'a>(
    selectors: &[Selector<'a>],
    active_lazy: Option<&HashSet<&str>>,
    mut compact_lengths: bool,
) -> Result<EventPaths<'a>> {
    let mut paths = Vec::new();
    let mut pending = Vec::new();
    let mut seen = HashSet::new();
    for &Selector { plan, node } in selectors {
        pending.push(node);
        seen.clear();
        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            let node = plan.nodes.get(id as usize).ok_or_else(invalid_plan)?;
            // Keep the actual value at a structural read, never a wire
            // truthiness placeholder. A repeat observes its collection;
            // there is no need to expand paths across all its items.
            for path in node
                .values
                .iter()
                .map(|p| p.path.as_slice())
                .chain(node.repeats.iter().map(|edge| edge.collection.as_slice()))
            {
                if path.is_empty() {
                    return Ok(EventPaths {
                        paths: Vec::new(),
                        compact_lengths: false,
                    });
                }
                paths.push(path);
            }
            pending.extend(
                node.scopes
                    .iter()
                    .filter(|edge| scope_is_active(edge, active_lazy))
                    .map(|edge| edge.node),
            );
        }
    }
    paths.sort_unstable_by(|a, b| a.iter().map(|f| &f.key).cmp(b.iter().map(|f| &f.key)));
    // Compare each candidate with the last retained prefix. An owned
    // argument must be moved just once even if a descendant is also read.
    paths.dedup_by(|candidate, prefix| {
        let covered = candidate.starts_with(prefix);
        // A whole `.length` includes `.length.child` for an object. Dropping
        // that redundant path must not turn a mixed string read into proof
        // that its length was the only argument.
        if covered
            && candidate.len() > prefix.len()
            && prefix.last().is_some_and(|f| f.key == "length")
        {
            compact_lengths = false;
        }
        covered
    });
    Ok(EventPaths {
        paths,
        compact_lengths,
    })
}

pub(crate) struct EventPaths<'a> {
    pub(crate) paths: Vec<&'a [HydrationReadField]>,
    pub(crate) compact_lengths: bool,
}

impl FrozenEventRoot {
    pub(crate) fn is_whole(&self) -> bool {
        self.paths.is_empty()
    }

    pub(crate) fn snapshot(&self, input: SnapshotInput<'_>) -> Value {
        snapshot_paths(&self.paths, input, self.compact_lengths)
    }

    pub(crate) fn unchanged(&self, input: &Value, frozen: &Value) -> bool {
        if self.is_whole() {
            return input == frozen;
        }
        self.paths.iter().enumerate().all(|(index, path)| {
            path_matches(
                input,
                frozen,
                path,
                self.compact_lengths && sole_length_path(&self.paths, index),
            )
        })
    }
}

pub(crate) fn snapshot_paths<P: AsRef<[HydrationReadField]>>(
    paths: &[P],
    mut input: SnapshotInput<'_>,
    compact_lengths: bool,
) -> Value {
    if let SnapshotInput::Owned(value) = input {
        project_owned_paths(value, paths, compact_lengths);
        return value.take();
    }
    let mut projected = Value::Null;
    if paths.is_empty() {
        input.store(&mut projected);
    } else {
        for (index, path) in paths.iter().enumerate() {
            input.reborrow().copy_path(
                path.as_ref(),
                &mut projected,
                compact_lengths && sole_length_path(paths, index),
            );
        }
    }
    projected
}

// Canonical paths are sorted and prefix-free. A length leaf is exclusive
// exactly when neither neighbor reads the same parent.
fn sole_length_path<P: AsRef<[HydrationReadField]>>(paths: &[P], index: usize) -> bool {
    let Some((last, parent)) = paths[index].as_ref().split_last() else {
        return false;
    };
    last.key == "length"
        && (index == 0 || !paths[index - 1].as_ref().starts_with(parent))
        && paths
            .get(index + 1)
            .is_none_or(|next| !next.as_ref().starts_with(parent))
}

fn string_length_shape(len: usize) -> Value {
    let mut fields = Map::with_capacity(1);
    fields.insert("length".into(), Value::from(len));
    Value::Object(fields)
}

impl<'a> ReadSelection<'a> {
    pub(crate) fn can_compact_strings(&self, key: &str) -> bool {
        // A closed lazy body may later read the actual string, not its length.
        self.roots
            .get(key)
            .is_some_and(|selectors| selectors.iter().all(|s| !s.plan.has_lazy_edges))
    }

    pub(crate) fn contains(&self, key: &str) -> bool {
        self.roots.contains_key(key)
    }

    pub(crate) fn paths_for(&self, key: &str) -> Result<Option<EventPaths<'a>>> {
        self.roots
            .get(key)
            .map(|selectors| self.prepare_paths(key, selectors))
            .transpose()
    }

    fn prepare_paths(&self, key: &str, selectors: &[Selector<'a>]) -> Result<EventPaths<'a>> {
        event_paths(
            selectors,
            self.active_lazy.as_ref(),
            self.can_compact_strings(key),
        )
    }

    pub(crate) fn take_present(
        &self,
        source: &mut Map<String, Value>,
        projected: &mut Map<String, Value>,
    ) -> Result<()> {
        if source.is_empty() {
            return Ok(());
        }
        for (&key, selectors) in &self.roots {
            if let Some((key, mut value)) = source.remove_entry(key) {
                let reads = self.prepare_paths(&key, selectors)?;
                projected.insert(
                    key,
                    snapshot_paths(
                        &reads.paths,
                        SnapshotInput::Owned(&mut value),
                        reads.compact_lengths,
                    ),
                );
            }
        }
        Ok(())
    }

    pub(crate) fn into_frozen(self) -> Result<Box<[FrozenEventRoot]>> {
        self.roots
            .iter()
            .map(|(&key, selectors)| {
                let reads = self.prepare_paths(key, selectors)?;
                Ok(FrozenEventRoot {
                    key: key.into(),
                    paths: reads.paths.into_iter().map(Box::from).collect(),
                    compact_lengths: reads.compact_lengths,
                })
            })
            .collect()
    }
}

pub(crate) enum SnapshotInput<'a> {
    Borrowed(&'a Value),
    Owned(&'a mut Value),
}

impl SnapshotInput<'_> {
    pub(crate) fn value(&self) -> &Value {
        match self {
            Self::Borrowed(value) => value,
            Self::Owned(value) => value,
        }
    }

    pub(crate) fn reborrow(&mut self) -> SnapshotInput<'_> {
        match self {
            Self::Borrowed(value) => SnapshotInput::Borrowed(value),
            Self::Owned(value) => SnapshotInput::Owned(value),
        }
    }

    pub(crate) fn child(self, index: impl serde_json::value::Index) -> Option<Self> {
        match self {
            Self::Borrowed(value) => value.get(index).map(Self::Borrowed),
            Self::Owned(value) => value.get_mut(index).map(Self::Owned),
        }
    }

    pub(crate) fn store(self, target: &mut Value) {
        match self {
            Self::Borrowed(value) => target.clone_from(value),
            Self::Owned(value) => *target = value.take(),
        }
    }

    fn copy_path(mut self, path: &[HydrationReadField], mut target: &mut Value, compact: bool) {
        for (position, field) in path.iter().enumerate() {
            match self.value() {
                Value::Object(_) => {
                    if target.is_null() {
                        *target = Value::Object(Map::new());
                    }
                    let Some(child) = self.child(field.key.as_str()) else {
                        return;
                    };
                    self = child;
                    target = &mut target[field.key.as_str()];
                }
                Value::Array(items) => {
                    if target.is_null() {
                        *target = Value::Array(vec![Value::Null; items.len()]);
                    }
                    let Some(index) = field.array_index.map(|i| i as usize) else {
                        return;
                    };
                    let Some(child) = self.child(index) else {
                        return;
                    };
                    self = child;
                    target = &mut target[index];
                }
                Value::String(text) if field.key == "length" && position + 1 == path.len() => {
                    if compact && compact_string_length(text.len()) {
                        *target = string_length_shape(text.len());
                    } else {
                        self.store(target);
                    }
                    return;
                }
                _ => return,
            }
        }
        self.store(target);
    }
}

// Compare the same structure we would copy. Do not clone a large authored
// leaf (or walk any unused sibling) to discover that a resume changed nothing.
fn path_matches(
    mut input: &Value,
    mut frozen: &Value,
    path: &[HydrationReadField],
    compact: bool,
) -> bool {
    for (position, field) in path.iter().enumerate() {
        let (next, prior) = match (input, frozen) {
            (Value::Object(a), Value::Object(b)) => (a.get(&field.key), b.get(&field.key)),
            (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
                let Some(index) = field.array_index else {
                    return true;
                };
                (a.get(index as usize), b.get(index as usize))
            }
            (Value::Array(_) | Value::Object(_), _) => return false,
            (Value::String(text), _) if field.key == "length" && position + 1 == path.len() => {
                if compact && compact_string_length(text.len()) {
                    return frozen.get("length") == Some(&Value::from(text.len()));
                }
                return input == frozen;
            }
            _ => return frozen.is_null() || (input.is_string() && input == frozen),
        };
        match (next, prior) {
            (Some(a), Some(b)) => (input, frozen) = (a, b),
            (None, None) => return true,
            _ => return false,
        }
    }
    input == frozen
}

pub(crate) fn has_narrow_roots<'a>(
    protocol: &WebUIProtocol,
    mut tags: impl Iterator<Item = &'a str>,
    whole: KeyView<'_>,
) -> bool {
    !protocol.hydration_reads.is_empty()
        && tags.any(|tag| {
            let owner = protocol
                .client_fragments
                .get(tag)
                .map_or(tag, |frag| frag.owner.as_str());
            protocol.hydration_reads.get(owner).is_some_and(|plan| {
                plan.roots.iter().any(|root| {
                    root.reaches_event && root.key != STATE_INJECT_KEY && !whole.contains(&root.key)
                })
            })
        })
}

pub(crate) fn select<'a, 'b>(
    protocol: &'a WebUIProtocol,
    tags: impl Iterator<Item = &'b str> + Clone,
    whole: KeyView<'_>,
    allow_lazy_updates: bool,
) -> Result<Option<ReadSelection<'a>>> {
    select_checked(protocol, tags, whole, allow_lazy_updates, |_| Ok(()))
}

pub(crate) fn select_checked<'a, 'b>(
    protocol: &'a WebUIProtocol,
    tags: impl Iterator<Item = &'b str> + Clone,
    whole: KeyView<'_>,
    allow_lazy_updates: bool,
    check_root_count: impl Fn(usize) -> Result<()>,
) -> Result<Option<ReadSelection<'a>>> {
    if !has_narrow_roots(protocol, tags.clone(), whole) {
        return Ok(None);
    }
    let mut plans = Vec::new();
    let mut owners = HashSet::new();
    // Updates may open a lazy region not present at its checkpoint.
    let mut active_lazy = (!allow_lazy_updates).then(HashSet::new);
    for tag in tags {
        let owner = match protocol.client_fragments.get_key_value(tag) {
            Some((id, fragment)) => {
                if fragment.entry {
                    if let Some(active) = &mut active_lazy {
                        active.insert(id.as_str());
                    }
                }
                fragment.owner.as_str()
            }
            None => tag,
        };
        if let Some((owner, plan)) = protocol.hydration_reads.get_key_value(owner) {
            if owners.insert(owner.as_str()) {
                plans.push(plan);
            }
        }
    }

    let mut roots = BTreeMap::<&str, Vec<Selector<'_>>>::new();
    let mut search = EventSearch::default();
    for plan in &plans {
        for root in &plan.roots {
            if root.reaches_event
                && root.key != STATE_INJECT_KEY
                && !whole.contains(&root.key)
                && (!plan.has_lazy_edges
                    || active_lazy.is_none()
                    || search.reaches_event(plan, root.node, active_lazy.as_ref())?)
            {
                roots.entry(&root.key).or_default();
                check_root_count(roots.len())?;
            }
        }
    }
    if roots.is_empty() {
        return Ok(None);
    }
    // Once a startup key is present, all reachable readers must see their paths.
    for plan in plans {
        for root in &plan.roots {
            if let Some(selectors) = roots.get_mut(root.key.as_str()) {
                selectors.push(Selector {
                    plan,
                    node: root.node,
                });
            }
        }
    }
    Ok(Some(ReadSelection { roots, active_lazy }))
}

#[derive(Default)]
struct EventSearch {
    pending: Vec<u32>,
    seen: HashSet<u32>,
}

impl EventSearch {
    fn reaches_event(
        &mut self,
        plan: &HydrationReadPlan,
        node: u32,
        active_lazy: Option<&HashSet<&str>>,
    ) -> Result<bool> {
        self.pending.clear();
        self.seen.clear();
        self.pending.push(node);
        while let Some(id) = self.pending.pop() {
            if !self.seen.insert(id) {
                continue;
            }
            let node = plan.nodes.get(id as usize).ok_or_else(invalid_plan)?;
            if node.reads_event {
                return Ok(true);
            }
            self.pending.extend(
                node.scopes
                    .iter()
                    .filter(|edge| scope_is_active(edge, active_lazy))
                    .map(|edge| edge.node),
            );
            self.pending
                .extend(node.repeats.iter().map(|edge| edge.item));
        }
        Ok(false)
    }
}

fn scope_is_active(edge: &HydrationReadScope, active_lazy: Option<&HashSet<&str>>) -> bool {
    edge.lazy_fragment.is_empty()
        || active_lazy.is_none_or(|ids| ids.contains(edge.lazy_fragment.as_str()))
}

#[cold]
#[inline(never)]
fn invalid_plan() -> HandlerError {
    HandlerError::Invariant("invalid compiled hydration read plan".to_owned())
}
