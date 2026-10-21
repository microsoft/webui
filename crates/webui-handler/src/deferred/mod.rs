// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

mod index;
mod scope;

use crate::{
    route_matcher, route_renderer, HandlerError, LoopBinding, RenderFragmentList,
    ResolvedRenderFragmentIndex, Result, VisibleLoopScope,
};
use index::{Step, HAS_DEFERRED};
use scope::{Scope, Values};
use serde_json::Value;
use std::borrow::Cow;
use std::collections::HashSet;
use webui_protocol::{
    web_ui_fragment::Fragment, WebUIFragmentFor, WebUIProtocol, WebUiFragmentRoute,
};

pub(crate) use index::DeferredIndex;
pub(crate) type DeferredSelection<'a> = HashSet<&'a str>;

const MAX_PLAN_STEPS: usize = 1_000_000;
const MAX_PLAN_DEPTH: usize = 1024;

pub(crate) struct SelectionInput<'protocol, 'state, 'index> {
    pub(crate) protocol: &'protocol WebUIProtocol,
    pub(crate) render: &'index ResolvedRenderFragmentIndex<'protocol>,
    pub(crate) index: &'protocol DeferredIndex,
    pub(crate) state: &'state Value,
    pub(crate) entry: &'protocol str,
    pub(crate) request_path: &'protocol str,
    pub(crate) routes: &'protocol route_matcher::CompiledRouteIndex,
}

enum Frame<'protocol, 'state> {
    Record {
        slot: usize,
        next: usize,
        matched: Option<(String, usize)>,
    },
    ComponentEnd,
    Component(&'protocol str),
    Repeat {
        repeat: &'protocol WebUIFragmentFor,
        items: &'state [Value],
        next: usize,
    },
    IterationEnd(VisibleLoopScope),
    RepeatEnd {
        name: &'protocol str,
        owned: Option<Value>,
        borrowed: Option<&'state Value>,
    },
    RouteEnd {
        base: Cow<'protocol, str>,
        children: &'protocol [WebUiFragmentRoute],
    },
}

struct Selection<'protocol, 'state, 'index> {
    input: SelectionInput<'protocol, 'state, 'index>,
    frames: Vec<Frame<'protocol, 'state>>,
    scopes: Vec<Scope<'protocol, 'state>>,
    loops: Vec<LoopBinding<'protocol, 'state>>,
    active: DeferredSelection<'protocol>,
    base: Cow<'protocol, str>,
    children: &'protocol [WebUiFragmentRoute],
}

pub(crate) fn select<'protocol>(
    input: SelectionInput<'protocol, '_, '_>,
) -> Result<DeferredSelection<'protocol>> {
    let slot = input
        .render
        .index(input.entry)
        .ok_or_else(|| HandlerError::MissingFragment(input.entry.to_owned()))?;
    if input.index.records[slot].flags & HAS_DEFERRED == 0 {
        return Ok(HashSet::new());
    }
    let mut selection = Selection {
        input,
        frames: Vec::with_capacity(16),
        scopes: vec![Scope::new(0)],
        loops: Vec::new(),
        active: HashSet::new(),
        base: Cow::Borrowed("/"),
        children: &[],
    };
    selection.enter_record(slot)?;
    let mut steps = 0;
    while let Some(frame) = selection.frames.pop() {
        steps += 1;
        if steps > MAX_PLAN_STEPS || selection.frames.len() > MAX_PLAN_DEPTH {
            return Err(selection_limit());
        }
        selection.advance(frame)?;
    }
    Ok(selection.active)
}

impl<'protocol, 'state> Selection<'protocol, 'state, '_> {
    fn values(&self) -> Values<'_, 'protocol, 'state> {
        Values {
            scope: &self.scopes[self.scopes.len() - 1],
            loops: &self.loops,
            state: self.input.state,
        }
    }

    fn record(&self, slot: usize) -> Result<RenderFragmentList<'protocol>> {
        self.input.render.list(slot).ok_or_else(|| {
            HandlerError::Invariant("deferred selection lost a compiled record".into())
        })
    }

    fn target(&self, id: &str) -> Result<usize> {
        self.input
            .render
            .index(id)
            .ok_or_else(|| HandlerError::MissingFragment(id.to_owned()))
    }

    fn relevant(&self, slot: usize) -> bool {
        self.input.index.relevant(slot, !self.children.is_empty())
    }

    fn enter_record(&mut self, slot: usize) -> Result<()> {
        if !self.relevant(slot) {
            return Ok(());
        }
        let record = self.record(slot)?;
        let matched = if record.has_routes {
            route_renderer::find_best_route_match(
                record.fragments,
                self.input.request_path,
                &self.base,
                self.input.routes,
            )
            .map(|(name, matched)| (name, matched.consumed_segments))
        } else {
            None
        };
        self.frames.push(Frame::Record {
            slot,
            next: 0,
            matched,
        });
        Ok(())
    }

    fn advance(&mut self, frame: Frame<'protocol, 'state>) -> Result<()> {
        match frame {
            Frame::Record {
                slot,
                next,
                matched,
            } => {
                let Some(step) = self.input.index.records[slot].steps.get(next).cloned() else {
                    return Ok(());
                };
                let consumed = match (&step, &matched) {
                    (Step::Route(position), Some((name, consumed))) => {
                        match self.record(slot)?.fragments[*position].fragment.as_ref() {
                            Some(Fragment::Route(route)) if route.fragment_id == *name => {
                                Some(*consumed)
                            }
                            _ => None,
                        }
                    }
                    _ => None,
                };
                self.frames.push(Frame::Record {
                    slot,
                    next: next + 1,
                    matched,
                });
                self.step(slot, step, consumed)
            }
            Frame::ComponentEnd => {
                self.scopes.pop();
                Ok(())
            }
            Frame::Component(id) => self.component(self.target(id)?, Scope::new(self.loops.len())),
            Frame::Repeat {
                repeat,
                items,
                next,
            } => self.repeat(repeat, items, next),
            Frame::IterationEnd(visible) => {
                self.loops.pop();
                let last = self.scopes.len() - 1;
                self.scopes[last].visible = visible;
                Ok(())
            }
            Frame::RepeatEnd {
                name,
                owned,
                borrowed,
            } => {
                let last = self.scopes.len() - 1;
                if let Some(value) = owned {
                    self.scopes[last].owned.insert(name.to_owned(), value);
                }
                if let Some(value) = borrowed {
                    self.scopes[last].borrowed.insert(name, value);
                }
                Ok(())
            }
            Frame::RouteEnd { base, children } => {
                self.base = base;
                self.children = children;
                Ok(())
            }
        }
    }

    fn step(&mut self, slot: usize, step: Step, consumed: Option<usize>) -> Result<()> {
        let record = self.record(slot)?;
        match step {
            Step::Component {
                position,
                attributes,
            } => {
                let Some(Fragment::Component(component)) =
                    record.fragments[position].fragment.as_ref()
                else {
                    return Err(invalid_plan());
                };
                let target = self.target(&component.fragment_id)?;
                if !self.relevant(target) {
                    return Ok(());
                }
                let mut scope = Scope::new(self.loops.len());
                for position in attributes {
                    if let Some(Fragment::Attribute(attribute)) =
                        record.fragments[position].fragment.as_ref()
                    {
                        if let Some(name) = record.component_attr_name(position) {
                            self.values().attribute(
                                attribute,
                                name,
                                self.input.protocol,
                                &mut scope,
                            )?;
                        }
                    }
                }
                self.component(target, scope)
            }
            Step::Condition(position) => {
                let fragment = record.fragments[position].fragment.as_ref();
                let (condition, deferred) = match fragment {
                    Some(Fragment::IfCond(condition)) => (condition, condition.deferred),
                    _ => return Err(invalid_plan()),
                };
                let target = self.target(&condition.fragment_id)?;
                if !deferred && !self.relevant(target) {
                    return Ok(());
                }
                let expression = condition.condition.as_ref().ok_or_else(invalid_plan)?;
                if self.values().condition(expression)? {
                    if deferred {
                        self.active.insert(condition.fragment_id.as_str());
                    }
                    self.enter_record(target)?;
                }
                Ok(())
            }
            Step::Repeat(position) => {
                let Some(Fragment::ForLoop(repeat)) = record.fragments[position].fragment.as_ref()
                else {
                    return Err(invalid_plan());
                };
                if !self.relevant(self.target(&repeat.fragment_id)?) {
                    return Ok(());
                }
                let items = self.values().collection(&repeat.collection)?;
                let last = self.scopes.len() - 1;
                let owned = self.scopes[last].owned.remove(repeat.item.as_str());
                let borrowed = self.scopes[last].borrowed.remove(&repeat.item);
                self.frames.push(Frame::RepeatEnd {
                    name: &repeat.item,
                    owned,
                    borrowed,
                });
                self.frames.push(Frame::Repeat {
                    repeat,
                    items,
                    next: 0,
                });
                Ok(())
            }
            Step::Route(position) => {
                let Some(Fragment::Route(route)) = record.fragments[position].fragment.as_ref()
                else {
                    return Err(invalid_plan());
                };
                if let Some(consumed) = consumed {
                    self.route(route, consumed)?;
                }
                Ok(())
            }
            Step::Outlet => self.outlet(),
        }
    }

    fn component(&mut self, slot: usize, scope: Scope<'protocol, 'state>) -> Result<()> {
        if !self.relevant(slot) {
            return Ok(());
        }
        self.scopes.push(scope);
        self.frames.push(Frame::ComponentEnd);
        self.enter_record(slot)
    }

    fn repeat(
        &mut self,
        repeat: &'protocol WebUIFragmentFor,
        items: &'state [Value],
        next: usize,
    ) -> Result<()> {
        let Some(item) = items.get(next) else {
            return Ok(());
        };
        self.frames.push(Frame::Repeat {
            repeat,
            items,
            next: next + 1,
        });
        let last = self.scopes.len() - 1;
        self.frames
            .push(Frame::IterationEnd(self.scopes[last].visible));
        self.loops.push(LoopBinding {
            name: &repeat.item,
            value: item,
        });
        self.scopes[last].visible.end = self.loops.len();
        self.enter_record(self.target(&repeat.fragment_id)?)
    }

    fn route(&mut self, route: &'protocol WebUiFragmentRoute, consumed: usize) -> Result<()> {
        let base = std::mem::replace(
            &mut self.base,
            Cow::Owned(route_matcher::compute_route_base(
                self.input.request_path,
                consumed,
            )),
        );
        let children = std::mem::replace(&mut self.children, &route.children);
        self.frames.push(Frame::RouteEnd { base, children });
        if !route.fragment_id.is_empty() {
            self.frames.push(Frame::Component(&route.fragment_id));
        }
        if !route.content_fragment_id.is_empty() {
            self.enter_record(self.target(&route.content_fragment_id)?)?;
        }
        Ok(())
    }

    fn outlet(&mut self) -> Result<()> {
        let children = std::mem::take(&mut self.children);
        let segments = route_matcher::split_request_path(self.input.request_path);
        let mut best = None;
        for route in children {
            if let Some(matched) = route_matcher::match_route_indexed_with_segments(
                self.input.routes,
                &route.path,
                &self.base,
                &segments,
                route.exact,
            ) {
                if best
                    .as_ref()
                    .is_none_or(|(_, previous): &(_, route_matcher::RouteMatch)| {
                        matched.specificity > previous.specificity
                    })
                {
                    best = Some((route, matched));
                }
            }
        }
        if let Some((route, matched)) = best {
            self.route(route, matched.consumed_segments)?;
        }
        Ok(())
    }
}

#[cold]
#[inline(never)]
fn invalid_plan() -> HandlerError {
    HandlerError::Invariant(
        "invalid compiled deferred-selection instruction; rebuild the protocol".into(),
    )
}

#[cold]
#[inline(never)]
fn selection_limit() -> HandlerError {
    HandlerError::Rendering(
        "deferred-selection limit exceeded; reduce nested or repeated deferred regions".into(),
    )
}
