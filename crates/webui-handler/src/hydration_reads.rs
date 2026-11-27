// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

mod shape;

use std::collections::{BTreeMap, HashSet};

use webui_protocol::{HydrationReadPlan, HydrationReadScope, WebUIProtocol};

use crate::{HandlerError, KeyView, Result, STATE_INJECT_KEY};

#[derive(Clone, Copy)]
struct Selector<'a> {
    plan: &'a HydrationReadPlan,
    node: u32,
}

pub(crate) struct ReadSelection<'a> {
    roots: BTreeMap<&'a str, Vec<Selector<'a>>>,
    active_lazy: Option<HashSet<&'a str>>,
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
