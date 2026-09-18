// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::borrow::Cow;
use std::collections::{btree_map, BTreeMap};
use std::sync::Arc;

use serde::ser::SerializeMap;
use serde::{Serialize, Serializer};
use serde_json::Value;
use webui_state::find_value_by_dotted_path_ref;
use yoke::Yoke;

#[cfg(test)]
mod provenance_tests;

/// Whether the immutable protocol can consume fragment-input lineage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Provenance {
    Omit,
    Track,
}

/// An immutable selection that pins only its original top-level state root.
#[derive(Clone, Debug)]
pub(crate) struct SharedValue {
    value: Yoke<&'static Value, Arc<Value>>,
    origin: Option<Arc<SourceOrigin>>,
}

#[derive(Debug)]
pub(crate) enum SourceOrigin {
    Root(Arc<Value>),
    Path { parent: Arc<Self>, path: Arc<str> },
    Item { parent: Arc<Self>, index: usize },
}

impl SharedValue {
    pub(crate) fn with_provenance(value: Value, provenance: Provenance) -> Self {
        let root = Arc::new(value);
        let origin = (provenance == Provenance::Track)
            .then(|| Arc::new(SourceOrigin::Root(Arc::clone(&root))));
        Self {
            value: Yoke::attach_to_cart(root, |value| value),
            origin,
        }
    }

    pub(crate) fn provenance(&self) -> Option<&Arc<SourceOrigin>> {
        self.origin.as_ref()
    }

    fn provenance_policy(&self) -> Provenance {
        if self.origin.is_some() {
            Provenance::Track
        } else {
            Provenance::Omit
        }
    }

    pub(crate) fn get(&self) -> &Value {
        self.value.get()
    }

    pub(crate) fn project(&self, path: &str) -> Option<Self> {
        self.project_with_path(path, || Arc::from(path))
    }

    pub(crate) fn project_with_path(
        &self,
        path: &str,
        capture_path: impl FnOnce() -> Arc<str>,
    ) -> Option<Self> {
        // The projection starts at the captured value, not at its backing root.
        let projected = self.value.try_map_project_cloned(|value, _| {
            match find_value_by_dotted_path_ref(path, value) {
                Some(Cow::Borrowed(child)) => Ok(child),
                Some(Cow::Owned(length)) => Err(Some(length)),
                None => Err(None),
            }
        });
        match projected {
            Ok(value) => Some(Self {
                value,
                origin: self.origin.as_ref().map(|origin| {
                    Arc::new(SourceOrigin::Path {
                        parent: Arc::clone(origin),
                        path: capture_path(),
                    })
                }),
            }),
            Err(length) => {
                length.map(|value| Self::with_provenance(value, self.provenance_policy()))
            }
        }
    }

    pub(crate) fn item(&self, index: usize) -> Option<Self> {
        self.value
            .try_map_project_cloned(|value, _| {
                (*value)
                    .as_array()
                    .and_then(|items| items.get(index))
                    .ok_or(())
            })
            .ok()
            .map(|value| Self {
                value,
                origin: self.origin.as_ref().map(|origin| {
                    Arc::new(SourceOrigin::Item {
                        parent: Arc::clone(origin),
                        index,
                    })
                }),
            })
    }
}

#[cfg(test)]
impl SharedValue {
    pub(crate) fn new(value: Value) -> Self {
        Self::with_provenance(value, Provenance::Track)
    }

    pub(crate) fn origin(&self) -> &Arc<SourceOrigin> {
        self.provenance()
            .unwrap_or_else(|| panic!("test value must retain provenance"))
    }

    pub(crate) fn project_shared(&self, path: Arc<str>) -> Option<Self> {
        self.project_with_path(&path, || Arc::clone(&path))
    }
}

/// Independently replaceable roots of a retained streaming snapshot.
#[derive(Debug, Default)]
pub(crate) struct SharedState {
    roots: BTreeMap<String, SharedValue>,
    scalar: Option<SharedValue>,
}

impl SharedState {
    pub(crate) fn from_owned_with_provenance(state: Value, provenance: Provenance) -> Self {
        match state {
            Value::Object(source) => Self {
                roots: source
                    .into_iter()
                    .map(|(key, value)| (key, SharedValue::with_provenance(value, provenance)))
                    .collect(),
                scalar: None,
            },
            value => Self {
                roots: BTreeMap::new(),
                scalar: Some(SharedValue::with_provenance(value, provenance)),
            },
        }
    }

    pub(crate) fn from_borrowed_with_provenance(state: &Value, provenance: Provenance) -> Self {
        Self::from_owned_with_provenance(state.clone(), provenance)
    }

    pub(crate) fn from_selected_owned_with_provenance(
        state: Value,
        keys: &[Box<str>],
        provenance: Provenance,
    ) -> Self {
        let Value::Object(mut source) = state else {
            return Self::default();
        };
        let mut selected = Self::default();
        for key in keys {
            if let Some((key, value)) = source.remove_entry(key.as_ref()) {
                selected
                    .roots
                    .insert(key, SharedValue::with_provenance(value, provenance));
            }
        }
        selected
    }

    pub(crate) fn from_selected_with_provenance(
        state: &Value,
        keys: &[Box<str>],
        provenance: Provenance,
    ) -> Self {
        let mut selected = Self::default();
        for key in keys {
            if let Some(value) = state.get(key.as_ref()) {
                selected.replace_borrowed(key, value, provenance);
            }
        }
        selected
    }

    /// Apply an object patch, retaining omitted and unchanged roots.
    pub(crate) fn overlay_owned_with_provenance(
        &mut self,
        state: Value,
        keys: Option<&[Box<str>]>,
        provenance: Provenance,
    ) -> bool {
        let Value::Object(mut source) = state else {
            return false;
        };
        let mut changed = self.scalar.take().is_some();
        match keys {
            Some(keys) => {
                for key in keys {
                    if let Some((key, value)) = source.remove_entry(key.as_ref()) {
                        changed |= self.replace_owned(key, value, provenance);
                    }
                }
            }
            None => {
                for (key, value) in source {
                    changed |= self.replace_owned(key, value, provenance);
                }
            }
        }
        changed
    }

    /// Apply a borrowed object patch, copying only changed roots once.
    pub(crate) fn overlay_with_provenance(
        &mut self,
        state: &Value,
        keys: Option<&[Box<str>]>,
        provenance: Provenance,
    ) -> bool {
        let Value::Object(source) = state else {
            return false;
        };
        let mut changed = self.scalar.take().is_some();
        match keys {
            Some(keys) => {
                for key in keys {
                    if let Some(value) = source.get(key.as_ref()) {
                        changed |= self.replace_borrowed(key, value, provenance);
                    }
                }
            }
            None => {
                for (key, value) in source {
                    changed |= self.replace_borrowed(key, value, provenance);
                }
            }
        }
        changed
    }

    pub(crate) fn get(&self, key: &str) -> Option<&Value> {
        self.roots.get(key).map(SharedValue::get)
    }

    pub(crate) fn capture(&self, path: &str) -> Option<SharedValue> {
        if let Some(value) = &self.scalar {
            return value.project(path);
        }
        match path.split_once('.') {
            Some((root, rest)) => self.roots.get(root)?.project(rest),
            None => self.roots.get(path).cloned(),
        }
    }

    pub(crate) fn clear(&mut self) {
        self.roots.clear();
        self.scalar = None;
    }

    fn replace_owned(&mut self, key: String, value: Value, provenance: Provenance) -> bool {
        if let Some(slot) = self.roots.get_mut(&key) {
            if slot.get() == &value {
                return false;
            }
            *slot = SharedValue::with_provenance(value, provenance);
        } else {
            self.roots
                .insert(key, SharedValue::with_provenance(value, provenance));
        }
        true
    }

    fn replace_borrowed(&mut self, key: &str, value: &Value, provenance: Provenance) -> bool {
        if let Some(slot) = self.roots.get_mut(key) {
            if slot.get() == value {
                return false;
            }
            *slot = SharedValue::with_provenance(value.clone(), provenance);
        } else {
            self.roots.insert(
                key.to_owned(),
                SharedValue::with_provenance(value.clone(), provenance),
            );
        }
        true
    }
}

#[cfg(test)]
impl SharedState {
    fn from_owned(state: Value) -> Self {
        Self::from_owned_with_provenance(state, Provenance::Track)
    }

    fn from_borrowed(state: &Value) -> Self {
        Self::from_borrowed_with_provenance(state, Provenance::Track)
    }

    fn from_selected_owned(state: Value, keys: &[Box<str>]) -> Self {
        Self::from_selected_owned_with_provenance(state, keys, Provenance::Track)
    }

    fn from_selected(state: &Value, keys: &[Box<str>]) -> Self {
        Self::from_selected_with_provenance(state, keys, Provenance::Track)
    }

    fn overlay_owned(&mut self, state: Value, keys: Option<&[Box<str>]>) -> bool {
        self.overlay_owned_with_provenance(state, keys, Provenance::Track)
    }

    fn overlay(&mut self, state: &Value, keys: Option<&[Box<str>]>) -> bool {
        self.overlay_with_provenance(state, keys, Provenance::Track)
    }
}

/// A read-only view of ordinary borrowed or retained streaming state.
///
/// Copying this view does not copy state. Values returned by [`Self::get`] and
/// [`Self::iter`] borrow the original backing for `'a`. Serialization preserves
/// the complete JSON value, including non-object state, without rebuilding a
/// JSON tree.
#[derive(Clone, Copy, Debug)]
pub struct StateView<'a> {
    source: StateSource<'a>,
}

#[derive(Clone, Copy, Debug)]
enum StateSource<'a> {
    Borrowed(&'a Value),
    Shared(&'a SharedState),
}

impl<'a> From<&'a Value> for StateView<'a> {
    /// Borrow a JSON value without allocating or cloning it.
    fn from(state: &'a Value) -> Self {
        Self {
            source: StateSource::Borrowed(state),
        }
    }
}

impl<'a> StateView<'a> {
    pub(crate) fn len(self) -> usize {
        match self.source {
            StateSource::Borrowed(Value::Object(state)) => state.len(),
            StateSource::Shared(state) => state.roots.len(),
            StateSource::Borrowed(_) => 0,
        }
    }

    pub(crate) fn is_shared(self) -> bool {
        matches!(self.source, StateSource::Shared(_))
    }
    pub(crate) fn shared(state: &'a SharedState) -> Self {
        Self {
            source: StateSource::Shared(state),
        }
    }

    /// Borrow an object's top-level property by its literal key.
    ///
    /// Missing properties and non-object state return `None`. A present JSON
    /// null is returned as `Some(&Value::Null)`.
    #[must_use]
    pub fn get(self, key: &str) -> Option<&'a Value> {
        match self.source {
            StateSource::Borrowed(state) => state.get(key),
            StateSource::Shared(state) => state.get(key),
        }
    }

    /// Iterate over borrowed top-level object entries without allocating.
    ///
    /// Non-object state produces an empty iterator.
    pub fn iter(self) -> impl Iterator<Item = (&'a str, &'a Value)> {
        match self.source {
            StateSource::Borrowed(Value::Object(state)) => StateIter::Borrowed(state.iter()),
            StateSource::Shared(state) => StateIter::Shared(state.roots.iter()),
            StateSource::Borrowed(_) => StateIter::Empty,
        }
    }

    /// Whether this view represents a JSON object.
    #[must_use]
    pub fn is_object(self) -> bool {
        match self.source {
            StateSource::Borrowed(state) => state.is_object(),
            StateSource::Shared(state) => state.scalar.is_none(),
        }
    }

    pub(crate) fn resolve(self, path: &str) -> Option<Cow<'a, Value>> {
        match self.source {
            StateSource::Borrowed(state) => find_value_by_dotted_path_ref(path, state),
            StateSource::Shared(state) => {
                if let Some(value) = &state.scalar {
                    return find_value_by_dotted_path_ref(path, value.get());
                }
                match path.split_once('.') {
                    Some((root, rest)) => find_value_by_dotted_path_ref(rest, state.get(root)?),
                    None => state.get(path).map(Cow::Borrowed),
                }
            }
        }
    }

    pub(crate) fn capture(self, path: &str) -> Option<SharedValue> {
        match self.source {
            StateSource::Borrowed(_) => None,
            StateSource::Shared(state) => state.capture(path),
        }
    }
}

impl Serialize for StateView<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.source {
            StateSource::Borrowed(state) => state.serialize(serializer),
            StateSource::Shared(state) => {
                if let Some(value) = &state.scalar {
                    return value.get().serialize(serializer);
                }
                let mut map = serializer.serialize_map(Some(state.roots.len()))?;
                for (key, value) in self.iter() {
                    map.serialize_entry(key, value)?;
                }
                map.end()
            }
        }
    }
}

enum StateIter<'a> {
    Borrowed(serde_json::map::Iter<'a>),
    Shared(btree_map::Iter<'a, String, SharedValue>),
    Empty,
}

impl<'a> Iterator for StateIter<'a> {
    type Item = (&'a str, &'a Value);

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Borrowed(iter) => iter.next().map(|(key, value)| (key.as_str(), value)),
            Self::Shared(iter) => iter.next().map(|(key, value)| (key.as_str(), value.get())),
            Self::Empty => None,
        }
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use webui_test_utils::test_json;

    #[test]
    fn borrowed_view_reads_without_copying() {
        let state = test_json!({"root": {"name": "Ada"}, "null": null, "root.name": 7});
        let view = StateView::from(&state);
        let copied = view;
        assert!(copied.is_object());
        assert!(std::ptr::eq(
            copied.get("root").expect("root"),
            &state["root"]
        ));
        assert_eq!(view.get("null"), Some(&Value::Null));
        assert_eq!(view.get("missing"), None);
        assert_eq!(view.get("root.name"), Some(&Value::from(7)));
        assert!(view.capture("root").is_none());
        let resolved = view.resolve("root.name").expect("name");
        assert!(matches!(resolved, Cow::Borrowed(_)));
        assert!(std::ptr::eq(resolved.as_ref(), &state["root"]["name"]));
    }

    #[test]
    fn views_iterate_and_serialize_the_same_ordered_object() {
        let original = test_json!({"z": [1, 2], "a": {"b": false}, "n": null});
        let shared = SharedState::from_borrowed(&original);
        for view in [StateView::from(&original), StateView::shared(&shared)] {
            assert_eq!(
                view.iter().map(|(key, _)| key).collect::<Vec<_>>(),
                ["a", "n", "z"]
            );
            for (key, value) in view.iter() {
                assert!(std::ptr::eq(value, view.get(key).expect("entry")));
            }
            assert_eq!(
                serde_json::to_string(&view).expect("serialize view"),
                serde_json::to_string(&original).expect("serialize original")
            );
        }
    }

    #[test]
    fn nonobjects_keep_their_complete_serialized_value() {
        for original in [
            Value::Null,
            Value::Bool(false),
            Value::from(0),
            Value::from(""),
            Value::from("é"),
            test_json!([{"nested": true}]),
        ] {
            let shared = SharedState::from_borrowed(&original);
            for view in [StateView::from(&original), StateView::shared(&shared)] {
                assert!(!view.is_object());
                assert_eq!(view.iter().next(), None);
                assert_eq!(view.get("length"), None);
                assert_eq!(serde_json::to_value(view).expect("serialize"), original);
                assert_eq!(
                    view.resolve("length"),
                    find_value_by_dotted_path_ref("length", &original)
                );
            }
        }
    }

    #[test]
    fn owned_inputs_preserve_nested_allocations() {
        for selected in [false, true] {
            let original = test_json!({"rows": [{"child": [1, 2, 3]}], "unused": [4]});
            let child = &original["rows"][0]["child"] as *const Value;
            let keys = [Box::<str>::from("rows")];
            let shared = if selected {
                SharedState::from_selected_owned(original, &keys)
            } else {
                SharedState::from_owned(original)
            };
            let captured = shared
                .capture("rows")
                .and_then(|rows| rows.item(0))
                .and_then(|row| row.project("child"))
                .expect("child");
            assert!(std::ptr::eq(captured.get(), child));
            assert_eq!(shared.get("unused").is_none(), selected);
        }
    }

    #[test]
    fn selected_snapshots_keep_only_requested_present_roots() {
        let original = test_json!({"keep": [1, 2], "skip": [3, 4]});
        let keys = [
            Box::<str>::from("keep"),
            Box::<str>::from("absent"),
            Box::<str>::from("keep"),
        ];
        let selected = SharedState::from_selected(&original, &keys);
        assert_eq!(selected.roots.len(), 1);
        assert_eq!(selected.get("keep"), Some(&original["keep"]));
        assert!(!std::ptr::eq(
            &selected.get("keep").expect("keep")[0],
            &original["keep"][0]
        ));
        for state in [
            SharedState::from_selected(&Value::Null, &keys),
            SharedState::from_selected_owned(Value::Null, &keys),
        ] {
            let view = StateView::shared(&state);
            assert!(view.is_object());
            assert_eq!(view.iter().next(), None);
        }
    }

    #[test]
    fn child_selections_share_the_original_root() {
        let root = SharedValue::new(test_json!({"rows": [{"child": {"leaf": 9}}]}));
        let rows = root.project("rows").expect("rows");
        let row = rows.item(0).expect("row");
        let child = row.project("child").expect("child");
        let leaf = child.project("leaf").expect("leaf");
        for descendant in [&rows, &row, &child, &leaf] {
            assert!(Arc::ptr_eq(
                root.value.backing_cart(),
                descendant.value.backing_cart()
            ));
        }
        assert!(std::ptr::eq(
            leaf.get(),
            &root.get()["rows"][0]["child"]["leaf"]
        ));
        assert!(rows.item(1).is_none());
        assert!(child.item(0).is_none());
        assert!(row.project("missing").is_none());
    }

    #[test]
    fn replacing_source_preserves_children_without_pinning_other_roots() {
        let mut state = SharedState::from_owned(test_json!({
            "root": {"child": {"leaf": "old"}},
            "unrelated": {"data": [1, 2]},
            "obsolete": [3, 4]
        }));
        let root_weak = Arc::downgrade(state.roots["root"].value.backing_cart());
        let unrelated_weak = Arc::downgrade(state.roots["unrelated"].value.backing_cart());
        let obsolete_weak = Arc::downgrade(state.roots["obsolete"].value.backing_cart());
        let child = state.capture("root.child").expect("child");
        let child_ptr = child.get() as *const Value;
        assert!(state.overlay_owned(
            test_json!({"root": {"child": {"leaf": "new"}}, "obsolete": []}),
            None
        ));
        assert!(obsolete_weak.upgrade().is_none());
        assert!(root_weak.upgrade().is_some());
        let leaf = child.project("leaf").expect("old leaf");
        assert!(std::ptr::eq(child.get(), child_ptr));
        assert_eq!(leaf.get(), "old");
        assert_eq!(state.get("root").expect("new root")["child"]["leaf"], "new");
        state.clear();
        assert!(unrelated_weak.upgrade().is_none());
        assert!(root_weak.upgrade().is_some());
        drop(child);
        assert!(root_weak.upgrade().is_some());
        drop(leaf);
        assert!(root_weak.upgrade().is_none());
        assert!(StateView::shared(&state).is_object());
        assert_eq!(StateView::shared(&state).iter().next(), None);
    }

    #[test]
    fn dropping_state_releases_unselected_backing() {
        let state = SharedState::from_owned(test_json!({"keep": {"child": 1}, "drop": [2]}));
        let kept_weak = Arc::downgrade(state.roots["keep"].value.backing_cart());
        let dropped_weak = Arc::downgrade(state.roots["drop"].value.backing_cart());
        let child = state.capture("keep.child").expect("child");
        drop(state);
        assert!(kept_weak.upgrade().is_some());
        assert!(dropped_weak.upgrade().is_none());
        drop(child);
        assert!(kept_weak.upgrade().is_none());
    }

    #[test]
    fn unchanged_overlays_keep_root_identity_and_omitted_values() {
        for owned in [false, true] {
            let mut state = SharedState::from_owned(test_json!({"same": [1, 2], "keep": true}));
            let same = state.capture("same").expect("same");
            let patch = test_json!({"same": [1, 2]});
            let changed = if owned {
                state.overlay_owned(patch, None)
            } else {
                state.overlay(&patch, None)
            };
            assert!(!changed);
            assert!(std::ptr::eq(
                same.get(),
                state.get("same").expect("same root")
            ));
            assert_eq!(state.get("keep"), Some(&Value::Bool(true)));
        }
    }

    #[test]
    fn selected_overlays_replace_only_changed_selected_roots() {
        for owned in [false, true] {
            let mut state = SharedState::from_owned(test_json!({"keep": 1, "skip": 2}));
            let skipped = state.capture("skip").expect("skip");
            let keys = [Box::<str>::from("keep"), Box::<str>::from("new")];
            let patch = test_json!({"keep": {"value": [3]}, "skip": 4, "new": null});
            let ptr = &patch["keep"]["value"] as *const Value;
            let changed = if owned {
                state.overlay_owned(patch, Some(&keys))
            } else {
                state.overlay(&patch, Some(&keys))
            };
            assert!(changed);
            assert_eq!(state.get("keep").expect("keep")["value"], test_json!([3]));
            assert_eq!(state.get("new"), Some(&Value::Null));
            assert!(std::ptr::eq(
                skipped.get(),
                state.get("skip").expect("skip root")
            ));
            if owned {
                assert!(std::ptr::eq(
                    &state.get("keep").expect("keep")["value"],
                    ptr
                ));
            }
        }
    }

    #[test]
    fn scalar_overlays_ignore_nonobjects_and_track_object_conversion() {
        for owned in [false, true] {
            let mut state = SharedState::from_owned(Value::from("old"));
            let weak = Arc::downgrade(state.scalar.as_ref().expect("scalar").value.backing_cart());
            assert!(!state.overlay(&Value::Null, None));
            assert!(!state.overlay_owned(Value::Bool(false), None));
            assert!(!StateView::shared(&state).is_object());
            let empty = test_json!({});
            let changed = if owned {
                state.overlay_owned(empty, None)
            } else {
                state.overlay(&empty, None)
            };
            assert!(changed);
            assert!(StateView::shared(&state).is_object());
            assert!(weak.upgrade().is_none());
        }
    }

    #[test]
    fn clear_releases_scalar_backing() {
        let mut state = SharedState::from_owned(test_json!([1, 2]));
        let weak = Arc::downgrade(state.scalar.as_ref().expect("array").value.backing_cart());
        state.clear();
        assert!(weak.upgrade().is_none());
        assert!(StateView::shared(&state).is_object());
    }

    #[test]
    fn falsy_inputs_are_present_captures() {
        let state = SharedState::from_owned(test_json!({
            "null": null, "false": false, "zero": 0, "empty": "", "array": [], "object": {}
        }));
        let view = StateView::shared(&state);
        for (key, value) in view.iter() {
            let selected = view.capture(key).expect("present input");
            assert!(std::ptr::eq(selected.get(), value));
        }
        assert!(view.capture("missing").is_none());
    }

    #[test]
    fn projected_lengths_are_terminal_synthetic_values() {
        let state = SharedState::from_owned(test_json!({
            "rows": [1, 2], "text": "é", "object": {"length": {"leaf": 8}, "0": 9}
        }));
        let view = StateView::shared(&state);
        for path in ["rows.length", "text.length"] {
            let length = view.capture(path).expect("length");
            assert_eq!(length.get(), &Value::from(2));
            assert!(matches!(view.resolve(path), Some(Cow::Owned(_))));
            assert!(length.project("trailing").is_none());
        }
        for path in ["rows.length.trailing", "text.length.trailing", "rows.0"] {
            assert!(view.capture(path).is_none());
            assert!(view.resolve(path).is_none());
        }
        assert_eq!(view.capture("object.length.leaf").expect("leaf").get(), 8);
        assert_eq!(
            view.capture("object.0").expect("numeric object key").get(),
            9
        );
        let rows = state.capture("rows").expect("rows");
        let length = rows.project("length").expect("length");
        assert!(!Arc::ptr_eq(
            rows.value.backing_cart(),
            length.value.backing_cart()
        ));
    }

    #[test]
    fn shared_handles_and_views_are_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<SharedValue>();
        assert_send_sync::<SharedState>();
        assert_send_sync::<StateView<'static>>();
    }

    #[test]
    fn shared_resolution_matches_borrowed_dotted_path_semantics() {
        let original = test_json!({
            "": {"": 1},
            "nested": {"": {"value": 2}, "length": {"value": 3}},
            "array": ["text"],
            "text": "é"
        });
        let state = SharedState::from_borrowed(&original);
        let view = StateView::shared(&state);
        for path in [
            "",
            ".",
            "nested..value",
            "nested.length.value",
            "array.length",
            "array.length.",
            "array.0",
            "text.length",
            "text.length.trailing",
            "missing",
        ] {
            let expected = find_value_by_dotted_path_ref(path, &original);
            assert_eq!(view.resolve(path), expected, "{path}");
            assert_eq!(
                view.capture(path).as_ref().map(SharedValue::get),
                expected.as_deref(),
                "{path}"
            );
        }
    }

    #[test]
    fn scalar_capture_length_does_not_pin_source_backing() {
        let state = SharedState::from_owned(test_json!([1, 2]));
        let weak = Arc::downgrade(state.scalar.as_ref().expect("array").value.backing_cart());
        let length = state.capture("length").expect("length");
        assert_eq!(length.get(), &Value::from(2));
        drop(state);
        assert!(weak.upgrade().is_none());
        assert_eq!(length.get(), &Value::from(2));
    }
}
