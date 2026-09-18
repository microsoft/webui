// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Compact scope transitions with separate, demand-allocated shared payloads.

use std::collections::HashMap;

use serde_json::Value;

use super::{RenderScopes, SharedAlias, SharedBindings};
use crate::{BorrowedScope, HandlerError, Result, VisibleLoopScope, WebUIProcessContext};

/// Lifetime-free part of a component or fragment-call scope transition.
pub(crate) struct SavedScope {
    locals: Option<HashMap<String, Value>>,
    shared: bool,
    borrowed_locals: bool,
    borrowed_alias: bool,
    visible: VisibleLoopScope,
}

pub(crate) struct SavedSharedScope {
    locals: Option<SharedBindings>,
    loops: SharedBindings,
    alias: SharedAlias,
}

impl SavedSharedScope {
    fn enter(scopes: &mut RenderScopes<'_, '_>, component: bool) -> bool {
        if scopes.loops.is_empty()
            && scopes.shared_alias.is_none()
            && (!component || scopes.locals.is_empty())
        {
            return false;
        }
        let saved = Self {
            locals: component.then(|| std::mem::take(&mut scopes.locals)),
            loops: std::mem::take(&mut scopes.loops),
            alias: scopes.shared_alias.take(),
        };
        scopes.shared_saves.push(saved);
        true
    }

    fn exit(scopes: &mut RenderScopes<'_, '_>, shared: bool, component: bool) -> Result<()> {
        if shared {
            let saved = scopes.shared_saves.pop().ok_or_else(scope_stack_error)?;
            scopes.loops = saved.loops;
            scopes.shared_alias = saved.alias;
            if component {
                let mut spent = std::mem::replace(
                    &mut scopes.locals,
                    saved.locals.ok_or_else(scope_stack_error)?,
                );
                spent.clear();
                if scopes.attrs.is_empty() && spent.capacity() > scopes.attrs.capacity() {
                    scopes.attrs = spent;
                }
            }
        } else {
            scopes.loops.clear();
            scopes.shared_alias = None;
            if component {
                scopes.locals.clear();
            }
        }
        Ok(())
    }
}

impl SavedScope {
    pub(crate) fn enter_component(context: &mut WebUIProcessContext<'_, '_, '_>) -> Option<Self> {
        if context.component_attrs.is_empty()
            && context.component_borrowed_attrs.inline_len == 0
            && context.scopes.attrs.is_empty()
            && context.local_vars.is_empty()
            && context.local_borrowed_vars.inline_len == 0
            && context.scopes.locals.is_empty()
            && context.scopes.loops.is_empty()
            && context.scopes.alias.is_none()
            && context.scopes.shared_alias.is_none()
            && context.visible_loop_scope.start == context.visible_loop_scope.end
        {
            context.collecting_component_attrs = false;
            None
        } else {
            Some(Self::enter(context, true))
        }
    }

    pub(crate) fn exit_empty_component(context: &mut WebUIProcessContext<'_, '_, '_>) {
        context.local_vars.clear();
        context.local_borrowed_vars.clear();
        context.scopes.locals.clear();
        context.scopes.loops.clear();
        context.scopes.alias = None;
        context.scopes.shared_alias = None;
        context.visible_loop_scope = VisibleLoopScope {
            start: context.loop_vars.len(),
            end: context.loop_vars.len(),
        };
        context.component_attrs.clear();
        context.component_borrowed_attrs.clear();
        context.scopes.attrs.clear();
        context.collecting_component_attrs = false;
    }

    pub(crate) fn enter(context: &mut WebUIProcessContext<'_, '_, '_>, component: bool) -> Self {
        let borrowed_alias = context.scopes.alias.take().map(|alias| {
            context.scopes.alias_saves.push(alias);
        });
        let mut saved = Self {
            locals: None,
            shared: SavedSharedScope::enter(&mut context.scopes, component),
            borrowed_locals: false,
            borrowed_alias: borrowed_alias.is_some(),
            visible: context.visible_loop_scope,
        };
        context.visible_loop_scope = VisibleLoopScope {
            start: context.loop_vars.len(),
            end: context.loop_vars.len(),
        };
        if component {
            saved.locals = Some(std::mem::replace(
                &mut context.local_vars,
                std::mem::replace(
                    &mut context.component_attrs,
                    crate::take_scope_map(&mut context.scope_pool),
                ),
            ));
            if !context.scopes.attrs.is_empty() {
                context.scopes.locals = std::mem::take(&mut context.scopes.attrs);
            }
            let borrowed = std::mem::replace(
                &mut context.local_borrowed_vars,
                std::mem::replace(
                    &mut context.component_borrowed_attrs,
                    crate::take_borrowed_scope(&mut context.borrowed_scope_pool),
                ),
            );
            if borrowed.inline_len != 0 {
                context.scopes.borrowed_saves.push(borrowed);
                saved.borrowed_locals = true;
            }
            context.collecting_component_attrs = false;
        }
        saved
    }

    pub(crate) fn exit(self, context: &mut WebUIProcessContext<'_, '_, '_>) -> Result<()> {
        context.visible_loop_scope = self.visible;
        SavedSharedScope::exit(&mut context.scopes, self.shared, self.locals.is_some())?;
        context.scopes.alias = if self.borrowed_alias {
            Some(
                context
                    .scopes
                    .alias_saves
                    .pop()
                    .ok_or_else(scope_stack_error)?,
            )
        } else {
            None
        };
        if let Some(locals) = self.locals {
            crate::recycle_scope_map(
                &mut context.scope_pool,
                std::mem::replace(&mut context.local_vars, locals),
            );
            let borrowed = if self.borrowed_locals {
                context
                    .scopes
                    .borrowed_saves
                    .pop()
                    .ok_or_else(scope_stack_error)?
            } else {
                BorrowedScope::default()
            };
            crate::recycle_borrowed_scope(
                &mut context.borrowed_scope_pool,
                std::mem::replace(&mut context.local_borrowed_vars, borrowed),
            );
            context.component_attrs.clear();
            context.component_borrowed_attrs.clear();
            context.scopes.attrs.clear();
            context.collecting_component_attrs = false;
        }
        Ok(())
    }
}

#[cold]
#[inline(never)]
fn scope_stack_error() -> HandlerError {
    HandlerError::Invariant("render scope stack was not balanced".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_view::SharedValue;
    use std::sync::Arc;
    use webui_test_utils::test_json;

    #[test]
    fn ordinary_scope_saves_keep_shared_payloads_unallocated() -> Result<()> {
        assert!(std::mem::size_of::<SavedScope>() <= 72);
        let mut scopes = RenderScopes::default();
        for component in [false, true] {
            assert!(!SavedSharedScope::enter(&mut scopes, component));
            SavedSharedScope::exit(&mut scopes, false, component)?;
            assert_eq!(scopes.shared_saves.capacity(), 0);
        }
        Ok(())
    }

    #[test]
    fn shared_saves_restore_nested_scopes_and_keep_fragment_owner_props() -> Result<()> {
        let mut scopes = RenderScopes::default();
        scopes
            .locals
            .insert("prop".into(), SharedValue::new(test_json!("owner")));
        scopes
            .loops
            .insert("item".into(), SharedValue::new(test_json!("caller")));
        scopes.shared_alias = Some(("row".into(), SharedValue::new(test_json!("alias"))));
        assert!(SavedSharedScope::enter(&mut scopes, false));
        assert!(scopes.loops.is_empty());
        assert!(scopes.shared_alias.is_none());
        assert_eq!(scopes.locals["prop"].get(), &test_json!("owner"));

        assert!(SavedSharedScope::enter(&mut scopes, true));
        assert!(scopes.locals.is_empty());
        let capacity = scopes.shared_saves.capacity();
        assert!(!SavedSharedScope::enter(&mut scopes, true));
        scopes
            .locals
            .insert("child".into(), SharedValue::new(test_json!("child")));
        scopes.shared_alias = Some(("inner".into(), SharedValue::new(test_json!("inner"))));
        SavedSharedScope::exit(&mut scopes, false, true)?;
        assert!(scopes.locals.is_empty());
        assert!(scopes.shared_alias.is_none());
        SavedSharedScope::exit(&mut scopes, true, true)?;
        assert_eq!(scopes.locals["prop"].get(), &test_json!("owner"));
        SavedSharedScope::exit(&mut scopes, true, false)?;
        assert_eq!(scopes.locals["prop"].get(), &test_json!("owner"));
        assert_eq!(scopes.loops["item"].get(), &test_json!("caller"));
        assert_eq!(
            scopes.shared_alias.as_ref().map(|(name, _)| name.as_ref()),
            Some("row")
        );
        assert!(scopes.shared_saves.is_empty());
        assert_eq!(scopes.shared_saves.capacity(), capacity);
        assert!(SavedSharedScope::exit(&mut scopes, true, false).is_err());
        Ok(())
    }

    #[test]
    fn shared_component_exit_recycles_spent_buckets_and_drops_captures() -> Result<()> {
        let mut scopes = RenderScopes::default();
        scopes
            .locals
            .insert("caller".into(), SharedValue::new(test_json!("caller")));
        let caller = &scopes.locals["caller"] as *const SharedValue;
        scopes
            .loops
            .insert("outer".into(), SharedValue::new(test_json!("loop")));
        scopes.shared_alias = Some(("row".into(), SharedValue::new(test_json!("alias"))));
        assert!(SavedSharedScope::enter(&mut scopes, true));
        let child = SharedValue::new(test_json!({"large": [1, 2, 3]}));
        let weak = Arc::downgrade(child.origin());
        let crate::state_view::SourceOrigin::Root(backing) = child.origin().as_ref() else {
            panic!("the captured prop must be a root");
        };
        let backing = Arc::downgrade(backing);
        scopes.locals = HashMap::with_capacity(8);
        scopes.locals.insert("child".into(), child);
        let capacity = scopes.locals.capacity();
        let bucket = &scopes.locals["child"] as *const SharedValue;

        SavedSharedScope::exit(&mut scopes, true, true)?;
        assert!(weak.upgrade().is_none());
        assert!(backing.upgrade().is_none());
        assert!(std::ptr::eq(&scopes.locals["caller"], caller));
        assert_eq!(scopes.loops["outer"].get(), "loop");
        assert_eq!(
            scopes.shared_alias.as_ref().map(|(name, _)| name.as_ref()),
            Some("row")
        );
        assert!(scopes.attrs.is_empty());
        assert_eq!(scopes.attrs.capacity(), capacity);
        scopes
            .attrs
            .insert("child".into(), SharedValue::new(test_json!("next")));
        assert!(std::ptr::eq(&scopes.attrs["child"], bucket));
        Ok(())
    }

    #[test]
    fn shared_component_recycling_preserves_partial_attrs_and_larger_empty_scratch() -> Result<()> {
        for partial in [false, true] {
            let mut scopes = RenderScopes::default();
            scopes
                .locals
                .insert("caller".into(), SharedValue::new(test_json!("caller")));
            assert!(SavedSharedScope::enter(&mut scopes, true));
            scopes.locals = HashMap::with_capacity(8);
            let child = SharedValue::new(test_json!("child"));
            let spent = Arc::downgrade(child.origin());
            scopes.locals.insert("child".into(), child);
            scopes.attrs = HashMap::with_capacity(if partial { 1 } else { 32 });
            scopes
                .attrs
                .insert("partial".into(), SharedValue::new(test_json!("keep")));
            let bucket = &scopes.attrs["partial"] as *const SharedValue;
            let capacity = scopes.attrs.capacity();
            if !partial {
                scopes.attrs.clear();
            }
            SavedSharedScope::exit(&mut scopes, true, true)?;
            assert!(spent.upgrade().is_none());
            assert_eq!(scopes.attrs.capacity(), capacity);
            if partial {
                assert_eq!(scopes.attrs["partial"].get(), "keep");
            } else {
                assert!(scopes.attrs.is_empty());
                scopes
                    .attrs
                    .insert("partial".into(), SharedValue::new(test_json!("next")));
            }
            assert!(std::ptr::eq(&scopes.attrs["partial"], bucket));
        }
        Ok(())
    }

    #[test]
    fn ordinary_component_exit_keeps_its_existing_local_scratch() -> Result<()> {
        let mut scopes = RenderScopes::default();
        scopes.locals = HashMap::with_capacity(8);
        let child = SharedValue::new(test_json!("child"));
        let weak = Arc::downgrade(child.origin());
        scopes.locals.insert("child".into(), child);
        let capacity = scopes.locals.capacity();
        SavedSharedScope::exit(&mut scopes, false, true)?;
        assert!(weak.upgrade().is_none());
        assert_eq!(scopes.locals.capacity(), capacity);
        assert_eq!(scopes.attrs.capacity(), 0);
        Ok(())
    }
}
