// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Compact input provenance for adopting suspended fragment calls in the browser.

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use serde::ser::SerializeTuple;
use serde::{Serialize, Serializer};
use serde_json::Value;

use crate::state_view::{SharedValue, SourceOrigin};
use crate::{HandlerError, Result};

#[derive(Default)]
pub(crate) struct FragmentSources {
    next_id: u32,
    known: HashMap<usize, (Weak<SourceOrigin>, u32)>,
    pending: Vec<SourceDefinition>,
    stack: Vec<Arc<SourceOrigin>>,
}

impl FragmentSources {
    pub(crate) fn capture(&mut self, value: &SharedValue) -> Result<u32> {
        let captured = self.capture_lineage(value);
        if captured.is_err() {
            // A partial walk must not leak retained origins into the next capture.
            self.stack.clear();
        }
        captured
    }

    fn capture_lineage(&mut self, value: &SharedValue) -> Result<u32> {
        let mut origin = value.provenance().ok_or_else(source_untracked_error)?;
        let mut parent_id = loop {
            let key = Arc::as_ptr(origin) as usize;
            if let Some((known, id)) = self.known.get(&key) {
                if known.ptr_eq(&Arc::downgrade(origin)) {
                    break Some(*id);
                }
            }
            self.stack.push(Arc::clone(origin));
            match origin.as_ref() {
                SourceOrigin::Root(_) => break None,
                SourceOrigin::Path { parent, .. } | SourceOrigin::Item { parent, .. } => {
                    origin = parent;
                }
            }
        };
        while let Some(origin) = self.stack.pop() {
            let id = self.next_id;
            self.next_id = id.checked_add(1).ok_or_else(source_limit_error)?;
            let definition = match origin.as_ref() {
                SourceOrigin::Root(root) => SourceDefinition::Root(id, Arc::clone(root)),
                SourceOrigin::Path { path, .. } => SourceDefinition::Path(
                    id,
                    parent_id.ok_or_else(source_parent_error)?,
                    path.clone(),
                ),
                SourceOrigin::Item { index, .. } => {
                    SourceDefinition::Item(id, parent_id.ok_or_else(source_parent_error)?, *index)
                }
            };
            self.known
                .insert(Arc::as_ptr(&origin) as usize, (Arc::downgrade(&origin), id));
            self.pending.push(definition);
            parent_id = Some(id);
        }
        parent_id.ok_or_else(source_parent_error)
    }

    pub(crate) fn pending(&self) -> &[SourceDefinition] {
        &self.pending
    }

    pub(crate) fn emitted(&mut self) {
        self.pending.clear();
        // Weak identities preserve deduplication without pinning completed inputs.
        self.known
            .retain(|_, (origin, _)| origin.strong_count() != 0);
    }

    pub(crate) fn clear(&mut self) {
        self.pending.clear();
        self.known.clear();
        self.stack.clear();
    }
}

pub(crate) enum SourceDefinition {
    Root(u32, Arc<Value>),
    Path(u32, u32, Arc<str>),
    Item(u32, u32, usize),
}

impl Serialize for SourceDefinition {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        match self {
            Self::Root(id, root) => {
                let mut tuple = serializer.serialize_tuple(3)?;
                tuple.serialize_element(id)?;
                tuple.serialize_element(&0)?;
                tuple.serialize_element(root.as_ref())?;
                tuple.end()
            }
            Self::Path(id, parent, path) => {
                let mut tuple = serializer.serialize_tuple(4)?;
                tuple.serialize_element(id)?;
                tuple.serialize_element(&1)?;
                tuple.serialize_element(parent)?;
                tuple.serialize_element(path.as_ref())?;
                tuple.end()
            }
            Self::Item(id, parent, index) => {
                let mut tuple = serializer.serialize_tuple(4)?;
                tuple.serialize_element(id)?;
                tuple.serialize_element(&2)?;
                tuple.serialize_element(parent)?;
                tuple.serialize_element(index)?;
                tuple.end()
            }
        }
    }
}

#[cold]
#[inline(never)]
fn source_untracked_error() -> HandlerError {
    HandlerError::Invariant("untracked input reached fragment provenance capture".to_owned())
}

#[cold]
#[inline(never)]
fn source_limit_error() -> HandlerError {
    HandlerError::Invariant(
        "fragment input provenance exceeds the response ID limit; reduce rendered input projections"
            .to_owned(),
    )
}

#[cold]
#[inline(never)]
fn source_parent_error() -> HandlerError {
    HandlerError::Invariant("fragment input provenance has no registered parent".to_owned())
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn roots_are_serialized_once_and_descendants_use_relative_projections() -> Result<()> {
        let root = SharedValue::new(json!({"children": [{"title": "OLD"}]}));
        let children = root.project("children").ok_or_else(source_parent_error)?;
        let child = children.item(0).ok_or_else(source_parent_error)?;
        let mut sources = FragmentSources::default();
        assert_eq!(sources.capture(&root)?, 0);
        sources.emitted();
        assert_eq!(sources.capture(&child)?, 2);
        assert_eq!(
            serde_json::to_value(sources.pending()).unwrap(),
            json!([[1, 1, 0, "children"], [2, 2, 1, 0]])
        );
        assert_eq!(sources.capture(&child)?, 2);
        assert_eq!(sources.pending().len(), 2);
        Ok(())
    }

    #[test]
    fn shared_ancestors_are_defined_once_across_records() -> Result<()> {
        let root = SharedValue::new(json!({"rows": [{"title": "A"}, {"title": "B"}]}));
        let rows = root.project("rows").ok_or_else(source_parent_error)?;
        let mut sources = FragmentSources::default();
        assert_eq!(
            sources.capture(&rows.item(0).ok_or_else(source_parent_error)?)?,
            2
        );
        sources.emitted();
        let second = sources.capture(&rows.item(1).ok_or_else(source_parent_error)?)?;
        assert_eq!(second, 3);
        // The root and the `rows` projection stay live, so only the new item is defined.
        assert_eq!(
            serde_json::to_value(sources.pending()).unwrap(),
            json!([[3, 2, 1, 1]])
        );
        Ok(())
    }

    #[test]
    fn exhausting_the_response_id_space_is_a_typed_error() {
        let root = SharedValue::new(json!({"title": "OLD"}));
        let mut sources = FragmentSources::default();
        sources.next_id = u32::MAX;
        let error = sources.capture(&root).unwrap_err();
        assert!(
            matches!(error, HandlerError::Invariant(ref message) if message.contains("ID limit"))
        );
        // A failed capture must not leave a half-walked lineage behind.
        assert!(sources.stack.is_empty());
        sources.next_id = 0;
        assert_eq!(sources.capture(&root).unwrap(), 0);
    }

    #[test]
    fn completed_inputs_are_released_after_their_definition_is_emitted() -> Result<()> {
        let root = SharedValue::new(json!({"title": "OLD"}));
        let weak = Arc::downgrade(root.origin());
        let SourceOrigin::Root(backing) = root.origin().as_ref() else {
            return Err(source_parent_error());
        };
        let backing = Arc::downgrade(backing);
        let mut sources = FragmentSources::default();
        sources.capture(&root)?;
        drop(root);
        assert!(weak.upgrade().is_none());
        assert!(backing.upgrade().is_some());
        assert_eq!(sources.pending().len(), 1);
        sources.emitted();
        assert!(sources.known.is_empty());
        assert!(sources.pending().is_empty());
        assert!(backing.upgrade().is_none());
        Ok(())
    }

    #[test]
    fn untracked_capture_is_rejected_before_mutating_source_identity() -> Result<()> {
        let tracked = SharedValue::new(json!({"title": "OLD"}));
        let untracked = SharedValue::with_provenance(
            json!({"title": "NEW"}),
            crate::state_view::Provenance::Omit,
        );
        for populated in [false, true] {
            let mut sources = FragmentSources::default();
            if populated {
                assert_eq!(sources.capture(&tracked)?, 0);
            }
            let definitions = serde_json::to_value(sources.pending()).unwrap();
            let known = sources.known.len();
            let next = sources.next_id;
            let error = sources.capture(&untracked).unwrap_err();
            assert!(matches!(
                error,
                HandlerError::Invariant(ref message) if message.contains("untracked input")
            ));
            assert_eq!(sources.next_id, next);
            assert_eq!(sources.known.len(), known);
            assert_eq!(
                serde_json::to_value(sources.pending()).unwrap(),
                definitions
            );
            assert!(sources.stack.is_empty());
            assert_eq!(sources.capture(&tracked)?, 0);
        }
        Ok(())
    }
}
