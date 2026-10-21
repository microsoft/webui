// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::{
    format_plain_json_scalar, resolve_borrowed_collection, resolve_state_backed_value,
    resolve_value_from_sources, BorrowedScope, HandlerError, LocalValueSources, LoopBinding,
    Result, ScalarBuffer, ScopeMap, VisibleLoopScope,
};
use serde_json::Value;
use std::borrow::Cow;
use webui_expressions::{evaluate_with_resolver, ExpressionError};
use webui_protocol::{
    web_ui_fragment::Fragment, ConditionExpr, WebUIFragmentAttribute, WebUIProtocol,
};

pub(super) struct Scope<'protocol, 'state> {
    pub(super) owned: ScopeMap,
    pub(super) borrowed: BorrowedScope<'protocol, 'state>,
    pub(super) visible: VisibleLoopScope,
}

impl Scope<'_, '_> {
    pub(super) fn new(loop_depth: usize) -> Self {
        Self {
            owned: ScopeMap::default(),
            borrowed: BorrowedScope::default(),
            visible: VisibleLoopScope {
                start: loop_depth,
                end: loop_depth,
            },
        }
    }
}

pub(super) struct Values<'context, 'protocol, 'state> {
    pub(super) scope: &'context Scope<'protocol, 'state>,
    pub(super) loops: &'context [LoopBinding<'protocol, 'state>],
    pub(super) state: &'state Value,
}

impl<'context, 'protocol, 'state> Values<'context, 'protocol, 'state> {
    fn locals(&self) -> LocalValueSources<'_, 'protocol, 'state> {
        LocalValueSources {
            owned: &self.scope.owned,
            borrowed: &self.scope.borrowed,
        }
    }

    fn resolve(&self, path: &str) -> Option<Cow<'_, Value>> {
        resolve_value_from_sources(
            path,
            self.loops,
            self.scope.visible,
            self.locals(),
            self.state,
        )
    }

    pub(super) fn condition(&self, condition: &ConditionExpr) -> Result<bool> {
        match evaluate_with_resolver(condition, |path| self.resolve(path)) {
            Ok(value) => Ok(value),
            Err(ExpressionError::MissingValue(_)) => Ok(false),
            Err(error) => Err(HandlerError::Evaluation(error.to_string())),
        }
    }

    pub(super) fn collection(&self, path: &str) -> Result<&'state [Value]> {
        if let Some(items) = resolve_borrowed_collection(
            path,
            self.loops,
            self.scope.visible,
            self.locals(),
            self.state,
        ) {
            return Ok(items);
        }
        match self.resolve(path) {
            None => Ok(&[]),
            Some(_) => Err(non_array_collection(path)),
        }
    }

    pub(super) fn attribute(
        &self,
        attribute: &'protocol WebUIFragmentAttribute,
        name: &'protocol str,
        protocol: &WebUIProtocol,
        target: &mut Scope<'protocol, 'state>,
    ) -> Result<()> {
        if attribute.attr_skip {
            return Ok(());
        }
        let owned = if let Some(condition) = &attribute.condition_tree {
            Some(Value::Bool(self.condition(condition)?))
        } else if !attribute.template.is_empty() {
            Some(Value::String(
                self.template_attribute(protocol, &attribute.template)?,
            ))
        } else if attribute.raw_value {
            Some(Value::String(attribute.value.clone()))
        } else if attribute.value.is_empty() {
            None
        } else if let Some(value) = resolve_state_backed_value(
            &attribute.value,
            self.loops,
            self.scope.visible,
            self.locals(),
            self.state,
        ) {
            target.owned.remove(name);
            target.borrowed.insert(name, value);
            return Ok(());
        } else {
            self.resolve(&attribute.value)
                .map(Cow::into_owned)
                .or_else(|| (!attribute.complex).then(|| Value::String(String::new())))
        };
        if let Some(value) = owned {
            target.borrowed.remove(name);
            target.owned.insert(name.to_owned(), value);
        }
        Ok(())
    }

    fn template_attribute(&self, protocol: &WebUIProtocol, id: &str) -> Result<String> {
        let list = protocol
            .fragments
            .get(id)
            .ok_or_else(|| HandlerError::MissingFragment(id.to_owned()))?;
        let mut value = String::with_capacity(crate::template_attr_capacity(&list.fragments));
        for fragment in &list.fragments {
            match fragment.fragment.as_ref() {
                Some(Fragment::Raw(raw)) => value.push_str(&raw.value),
                Some(Fragment::Signal(signal)) => {
                    if let Some(resolved) = self.resolve(&signal.value) {
                        match resolved.as_ref() {
                            Value::String(text) => value.push_str(text),
                            other => {
                                let mut scratch = ScalarBuffer::new();
                                if let Some(text) = format_plain_json_scalar(&mut scratch, other) {
                                    value.push_str(text);
                                } else {
                                    value.push_str(&other.to_string());
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(value)
    }
}

#[cold]
#[inline(never)]
fn non_array_collection(path: &str) -> HandlerError {
    HandlerError::TypeError(format!("Collection '{path}' is not a state-backed array"))
}
