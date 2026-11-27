// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::HashMap;

use super::{
    condition_plan, CompiledAttrBinding, CompiledAttrPart, CompiledRepeat, EventArg, EventBinding,
    TemplateMeta, TemplateSectionMeta,
};
use crate::{diagnostic::codes, Diagnostic, Result};
use webui_protocol::{
    HydrationReadField, HydrationReadNode, HydrationReadPath, HydrationReadPlan,
    HydrationReadRepeat, HydrationReadRoot, HydrationReadScope,
};

const MAX_PLAN_ENTRIES: usize = 262_144;

type Symbol<'a> = (usize, &'a str);

#[derive(Clone, Copy)]
struct Incoming<'a> {
    section: usize,
    repeat: Option<&'a CompiledRepeat>,
    lazy: Option<&'a str>,
}

impl<'a> Incoming<'a> {
    fn source(self, name: &'a str) -> (Symbol<'a>, Option<&'a str>) {
        if let Some(repeat) = self.repeat.filter(|repeat| repeat.item_var == name) {
            let (root, path) = split_path(&repeat.collection);
            return ((self.section, root), Some(path));
        }
        ((self.section, name), None)
    }
}

struct Compiler<'a> {
    incoming: Vec<Vec<Incoming<'a>>>,
    ids: HashMap<Symbol<'a>, u32>,
    symbols: Vec<Symbol<'a>>,
    nodes: Vec<HydrationReadNode>,
    remaining: usize,
}

pub(super) fn compile(owner: &str, meta: &TemplateMeta) -> Result<HydrationReadPlan> {
    let sections = || std::iter::once(&meta.root).chain(&meta.blocks);
    let mut compiler = Compiler {
        incoming: vec![Vec::new(); 1 + meta.blocks.len()],
        ids: HashMap::new(),
        symbols: Vec::new(),
        nodes: Vec::new(),
        remaining: MAX_PLAN_ENTRIES,
    };
    let lazy_ids: HashMap<_, _> = meta
        .lazy
        .iter()
        .map(|entry| (entry.block_index, entry.id.as_str()))
        .collect();
    for (index, section) in sections().enumerate() {
        for (position, (_, block)) in section.conditionals.iter().enumerate() {
            let lazy = section
                .is_lazy_condition(position)
                .then(|| lazy_ids.get(block).copied())
                .flatten();
            compiler.incoming[block + 1].push(Incoming {
                section: index,
                repeat: None,
                lazy,
            });
        }
        for repeat in &section.repeats {
            compiler.incoming[repeat.block_index + 1].push(Incoming {
                section: index,
                repeat: Some(repeat),
                lazy: None,
            });
        }
        compiler
            .add_section(index, section)
            .ok_or_else(|| limit_error(owner))?;
    }
    compiler
        .add_events(0, &meta.root_events)
        .ok_or_else(|| limit_error(owner))?;
    compiler.finish().ok_or_else(|| limit_error(owner))
}

impl<'a> Compiler<'a> {
    fn charge(&mut self) -> Option<()> {
        self.remaining = self.remaining.checked_sub(1)?;
        Some(())
    }

    fn node(&mut self, symbol: Symbol<'a>) -> Option<u32> {
        if let Some(&id) = self.ids.get(&symbol) {
            return Some(id);
        }
        self.charge()?;
        let id = u32::try_from(self.nodes.len()).ok()?;
        self.ids.insert(symbol, id);
        self.symbols.push(symbol);
        self.nodes.push(HydrationReadNode::default());
        Some(id)
    }

    fn add_path(
        &mut self,
        symbol: Symbol<'a>,
        path: &str,
        truthy: bool,
        event: bool,
    ) -> Option<()> {
        self.charge()?;
        let id = self.node(symbol)?;
        let node = &mut self.nodes[id as usize];
        node.values.push(HydrationReadPath {
            path: fields(path),
            truthy,
        });
        node.reads_event |= event;
        Some(())
    }

    fn read(&mut self, section: usize, path: &'a str, truthy: bool, event: bool) -> Option<()> {
        if path.is_empty() {
            return Some(());
        }
        let (root, suffix) = split_path(path);
        self.add_path((section, root), suffix, truthy, event)
    }

    fn add_events(&mut self, section: usize, events: &'a [EventBinding]) -> Option<()> {
        for (_, _, args) in events {
            for arg in args {
                if let EventArg::Path(path) = arg {
                    self.read(section, path, false, true)?;
                }
            }
        }
        Some(())
    }

    fn add_parts(&mut self, section: usize, parts: &'a [CompiledAttrPart]) -> Option<()> {
        for part in parts {
            if let CompiledAttrPart::Dynamic(path) = part {
                self.read(section, path, false, false)?;
            }
        }
        Some(())
    }

    fn add_condition(
        &mut self,
        section: usize,
        condition: &'a webui_protocol::ConditionExpr,
    ) -> Option<()> {
        condition_plan::visit_read_paths(condition, |path, truthy| {
            self.read(section, path, truthy, false).is_some()
        })
        .then_some(())
    }

    fn add_section(&mut self, index: usize, section: &'a TemplateSectionMeta) -> Option<()> {
        self.add_events(index, &section.events)?;
        for (_, parts, _) in &section.text_runs {
            self.add_parts(index, parts)?;
        }
        for attr in &section.attr_bindings {
            match attr {
                CompiledAttrBinding::Simple { value, .. }
                | CompiledAttrBinding::Complex { value, .. } => {
                    self.read(index, value, false, false)?;
                }
                CompiledAttrBinding::Boolean { condition, .. } => {
                    self.add_condition(index, condition)?;
                }
                CompiledAttrBinding::Template { parts, .. } => self.add_parts(index, parts)?,
            }
        }
        for (condition, _) in &section.conditionals {
            self.add_condition(index, condition)?;
        }
        for repeat in &section.repeats {
            let symbol = (repeat.block_index + 1, repeat.item_var.as_str());
            if let Some(key) = &repeat.key_path {
                self.add_path(symbol, key, false, false)?;
            } else {
                // The collection's positions matter even if no item fields are read.
                self.node(symbol)?;
            }
        }
        Some(())
    }

    fn finish(mut self) -> Option<HydrationReadPlan> {
        let mut has_lazy_edges = false;
        let mut cursor = 0;
        while let Some(&(section, name)) = self.symbols.get(cursor) {
            let child = u32::try_from(cursor).ok()?;
            for index in 0..self.incoming[section].len() {
                let edge = self.incoming[section][index];
                let (symbol, collection) = edge.source(name);
                self.charge()?;
                let from = self.node(symbol)?;
                let node = &mut self.nodes[from as usize];
                if let Some(collection) = collection {
                    node.repeats.push(HydrationReadRepeat {
                        collection: fields(collection),
                        item: child,
                    });
                } else {
                    has_lazy_edges |= edge.lazy.is_some();
                    node.scopes.push(HydrationReadScope {
                        node: child,
                        lazy_fragment: edge.lazy.unwrap_or_default().to_owned(),
                    });
                }
            }
            cursor += 1;
        }
        let reaches_event = self.event_reachability();
        let mut roots: Vec<_> = self
            .symbols
            .iter()
            .enumerate()
            .filter(|(_, (section, _))| *section == 0)
            .map(|(id, (_, name))| {
                Some(HydrationReadRoot {
                    key: (*name).to_owned(),
                    node: u32::try_from(id).ok()?,
                    reaches_event: reaches_event[id],
                })
            })
            .collect::<Option<_>>()?;
        roots.sort_unstable_by(|a, b| a.key.cmp(&b.key));
        for node in &mut self.nodes {
            node.values.sort_unstable_by(|a, b| {
                a.path
                    .iter()
                    .map(|field| field.key.as_str())
                    .cmp(b.path.iter().map(|field| field.key.as_str()))
                    .then_with(|| a.truthy.cmp(&b.truthy))
            });
            node.values.dedup();
        }
        Some(HydrationReadPlan {
            nodes: self.nodes,
            roots,
            has_lazy_edges,
        })
    }

    fn event_reachability(&self) -> Vec<bool> {
        let mut reaching = vec![false; self.nodes.len()];
        let mut pending = Vec::new();
        for (id, node) in self.nodes.iter().enumerate() {
            if node.reads_event {
                reaching[id] = true;
                pending.push(id);
            }
        }
        while let Some(id) = pending.pop() {
            let (section, name) = self.symbols[id];
            for edge in &self.incoming[section] {
                let (symbol, _) = edge.source(name);
                let parent = self.ids[&symbol] as usize;
                if !std::mem::replace(&mut reaching[parent], true) {
                    pending.push(parent);
                }
            }
        }
        reaching
    }
}

fn split_path(path: &str) -> (&str, &str) {
    path.split_once('.').unwrap_or((path, ""))
}

fn fields(path: &str) -> Vec<HydrationReadField> {
    if path.is_empty() {
        return Vec::new();
    }
    path.split('.')
        .map(|key| {
            let array_index = (key.as_bytes().first().is_some_and(u8::is_ascii_digit)
                && !(key.len() > 1 && key.starts_with('0')))
            .then(|| key.parse::<u32>().ok().filter(|&index| index != u32::MAX))
            .flatten();
            HydrationReadField {
                key: key.to_owned(),
                array_index,
            }
        })
        .collect()
}

#[cold]
#[inline(never)]
fn limit_error(owner: &str) -> crate::ParserError {
    Diagnostic::error("compiled template hydration dependencies exceed the plan limit")
        .code(codes::HYDRATION_READ_LIMIT)
        .component(owner)
        .help("split the template or shared repeats into smaller components")
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write;

    #[test]
    fn shared_repeat_read_plan_grows_with_graph_facts_not_scope_combinations() -> Result<()> {
        const DEPTH: usize = 18;
        let mut markup = String::new();
        for level in 0..DEPTH {
            let _ = write!(
                markup,
                r#"<for id="shared-{level}" each="v{level} in left{level}">"#
            );
        }
        let _ = write!(
            markup,
            r#"<button @click="{{select(v0.id, v{}.id)}}">Select</button>"#,
            DEPTH - 1
        );
        for level in (0..DEPTH).rev() {
            let _ = write!(
                markup,
                r#"</for><for id="shared-{level}" each="v{level} in right{level}" />"#,
            );
        }
        let meta =
            super::super::compile_to_metadata("test-graph", markup.as_str().into(), Vec::new())?;
        let plan = compile("test-graph", &meta)?;
        let selected: Vec<_> = plan
            .roots
            .iter()
            .filter(|root| root.reaches_event)
            .map(|root| root.key.as_str())
            .collect();
        assert_eq!(selected, ["left0", "left17", "right0", "right17"]);
        assert!(plan.nodes.len() <= 4 * DEPTH * DEPTH);
        Ok(())
    }
}
