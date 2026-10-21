// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;
use webui_protocol::{web_ui_fragment::Fragment, FragmentList, WebUIProtocol, WebUiFragmentRoute};

pub(super) const HAS_DEFERRED: u8 = 1;
pub(super) const HAS_OUTLET: u8 = 2;

pub(crate) struct DeferredIndex {
    pub(super) records: Box<[RecordPlan]>,
}

pub(super) struct RecordPlan {
    pub(super) flags: u8,
    pub(super) steps: Box<[Step]>,
}

#[derive(Clone)]
pub(super) enum Step {
    Component {
        position: usize,
        attributes: Range<usize>,
    },
    Condition(usize),
    Repeat(usize),
    Route(usize),
    Outlet,
}

impl DeferredIndex {
    pub(crate) fn new(
        protocol: &WebUIProtocol,
        ids: &[Arc<str>],
        slots: &HashMap<Arc<str>, u32>,
    ) -> Option<Self> {
        if !protocol.fragments.values().any(|list| list.fragments.iter().any(|fragment| {
            matches!(fragment.fragment.as_ref(), Some(Fragment::IfCond(condition)) if condition.deferred)
        })) {
            return None;
        }

        let mut flags = vec![0u8; ids.len()];
        let mut reverse = vec![Vec::new(); ids.len()];
        let mut targets = Vec::new();
        for (slot, id) in ids.iter().enumerate() {
            let Some(list) = protocol.fragments.get(id.as_ref()) else {
                continue;
            };
            targets.clear();
            for fragment in &list.fragments {
                match fragment.fragment.as_ref() {
                    Some(Fragment::IfCond(condition)) => {
                        if condition.deferred {
                            flags[slot] |= HAS_DEFERRED;
                        }
                        targets.push(condition.fragment_id.as_str());
                    }
                    Some(Fragment::ForLoop(repeat)) => targets.push(repeat.fragment_id.as_str()),
                    Some(Fragment::Component(component)) => {
                        targets.push(component.fragment_id.as_str())
                    }
                    Some(Fragment::Route(route)) => route_targets(route, &mut targets),
                    Some(Fragment::Outlet(_)) => flags[slot] |= HAS_OUTLET,
                    _ => {}
                }
            }
            for target in &targets {
                if let Some(&child) = slots.get(*target) {
                    reverse[child as usize].push(slot);
                }
            }
        }
        propagate(&mut flags, &reverse);
        let records = ids
            .iter()
            .zip(flags)
            .map(|(id, flags)| {
                let steps = protocol
                    .fragments
                    .get(id.as_ref())
                    .filter(|_| flags != 0)
                    .map_or_else(Vec::new, compile_steps);
                RecordPlan {
                    flags,
                    steps: steps.into_boxed_slice(),
                }
            })
            .collect();
        Some(Self { records })
    }

    pub(crate) fn relevant(&self, slot: usize, has_route_children: bool) -> bool {
        self.records.get(slot).is_some_and(|record| {
            record.flags & HAS_DEFERRED != 0
                || (has_route_children && record.flags & HAS_OUTLET != 0)
        })
    }
}

fn route_targets<'a>(route: &'a WebUiFragmentRoute, targets: &mut Vec<&'a str>) {
    let mut pending = vec![route];
    while let Some(route) = pending.pop() {
        targets.push(route.fragment_id.as_str());
        if !route.content_fragment_id.is_empty() {
            targets.push(route.content_fragment_id.as_str());
        }
        pending.extend(&route.children);
    }
}

fn propagate(flags: &mut [u8], reverse: &[Vec<usize>]) {
    let mut pending: Vec<usize> = flags
        .iter()
        .enumerate()
        .filter_map(|(slot, flags)| (*flags != 0).then_some(slot))
        .collect();
    while let Some(child) = pending.pop() {
        for &parent in &reverse[child] {
            let next = flags[parent] | flags[child];
            if next != flags[parent] {
                flags[parent] = next;
                pending.push(parent);
            }
        }
    }
}

fn compile_steps(list: &FragmentList) -> Vec<Step> {
    let mut steps = Vec::new();
    let mut attributes = None;
    for (position, fragment) in list.fragments.iter().enumerate() {
        match fragment.fragment.as_ref() {
            Some(Fragment::Attribute(attribute)) if attribute.attr_start => {
                attributes = Some(position)
            }
            Some(Fragment::Component(_)) => {
                steps.push(Step::Component {
                    position,
                    attributes: attributes.take().unwrap_or(position)..position,
                });
            }
            Some(Fragment::IfCond(_)) => steps.push(Step::Condition(position)),
            Some(Fragment::ForLoop(_)) => steps.push(Step::Repeat(position)),
            Some(Fragment::Route(_)) => steps.push(Step::Route(position)),
            Some(Fragment::Outlet(_)) => steps.push(Step::Outlet),
            _ => {}
        }
    }
    steps
}
