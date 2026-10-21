// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::{
    emit_js_string, emit_json_template_section, ConditionFunctionEmitter, TemplateMeta,
    TemplateSectionMeta,
};
use crate::plugin::LazyTemplateArtifact;
use std::collections::HashSet;
use webui_protocol::{ClientFragmentData, StateProjectionMode};

pub(super) enum Reference {
    Inline(usize),
    Local(usize),
    External(String),
}

pub(super) struct Plan {
    pub(super) references: Vec<Reference>,
    bundles: Vec<Vec<usize>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Owner {
    Unassigned,
    Eager,
    Region(usize),
    Shared,
}

impl Plan {
    pub(super) fn new(owner: &str, meta: &TemplateMeta) -> Option<Self> {
        if meta.lazy.is_empty() {
            return None;
        }
        let mut eager = vec![false; meta.blocks.len()];
        let mut pending = Vec::new();
        push_ordinary(&meta.root, &mut pending);
        while let Some(index) = pending.pop() {
            if eager[index] {
                continue;
            }
            eager[index] = true;
            push_ordinary(&meta.blocks[index], &mut pending);
        }
        let mut owners: Vec<Owner> = eager
            .iter()
            .map(|eager| {
                if *eager {
                    Owner::Eager
                } else {
                    Owner::Unassigned
                }
            })
            .collect();
        let mut visited = vec![usize::MAX; meta.blocks.len()];
        for (region, declaration) in meta.lazy.iter().enumerate() {
            pending.push(declaration.block_index);
            while let Some(index) = pending.pop() {
                if owners[index] == Owner::Eager || visited[index] == region {
                    continue;
                }
                visited[index] = region;
                owners[index] = match owners[index] {
                    Owner::Unassigned => Owner::Region(region),
                    Owner::Region(previous) if previous == region => Owner::Region(region),
                    _ => Owner::Shared,
                };
                push_ordinary(&meta.blocks[index], &mut pending);
            }
        }
        let mut next = 0;
        let mut bundles = vec![Vec::new(); meta.lazy.len()];
        let mut references = Vec::with_capacity(meta.blocks.len());
        for (index, owner_kind) in owners.into_iter().enumerate() {
            references.push(match owner_kind {
                Owner::Eager => {
                    let inline = next;
                    next += 1;
                    Reference::Inline(inline)
                }
                Owner::Region(region) if meta.lazy[region].block_index != index => {
                    let local = bundles[region].len();
                    bundles[region].push(index);
                    Reference::Local(local)
                }
                _ => Reference::External(crate::lazy::block_id(owner, index)),
            });
        }
        for declaration in &meta.lazy {
            references[declaration.block_index] = Reference::External(declaration.id.clone());
        }
        Some(Self {
            references,
            bundles,
        })
    }

    pub(super) fn is_eager(&self, index: usize) -> bool {
        matches!(self.references[index], Reference::Inline(_))
    }

    pub(super) fn emit(&self, owner: &str, meta: &TemplateMeta) -> Vec<LazyTemplateArtifact> {
        let mut emitted = Vec::new();
        for (index, reference) in self.references.iter().enumerate() {
            let Reference::External(id) = reference else {
                continue;
            };
            let section = &meta.blocks[index];
            let declaration = meta
                .lazy
                .iter()
                .enumerate()
                .find(|(_, entry)| entry.block_index == index);
            let local_blocks = declaration
                .map(|(region, _)| self.bundles[region].as_slice())
                .unwrap_or_default();
            let mut functions = ConditionFunctionEmitter::new(64);
            let local_html: usize = local_blocks
                .iter()
                .map(|index| meta.blocks[*index].html.len())
                .sum();
            let mut json = String::with_capacity(section.html.len() + local_html + 128);
            json.push('{');
            emit_json_template_section(section, &mut json, &mut functions, Some(self));
            json.push_str(",\"v\":3,\"f\":");
            emit_js_string(id, &mut json);
            json.push_str(",\"o\":");
            emit_js_string(owner, &mut json);
            if declaration.is_some() {
                json.push_str(",\"e\":1");
            }
            if !local_blocks.is_empty() {
                json.push_str(",\"b\":[");
                for (position, index) in local_blocks.iter().enumerate() {
                    if position != 0 {
                        json.push(',');
                    }
                    json.push('{');
                    emit_json_template_section(
                        &meta.blocks[*index],
                        &mut json,
                        &mut functions,
                        Some(self),
                    );
                    json.push('}');
                }
                json.push(']');
            }
            let dependencies = self.dependencies(section, meta);
            json.push('}');
            emitted.push(LazyTemplateArtifact {
                id: id.clone(),
                data: ClientFragmentData {
                    owner: owner.to_owned(),
                    name: declaration
                        .map_or("", |(_, entry)| entry.name.as_str())
                        .to_owned(),
                    template_json: json,
                    template_functions: functions.finish(),
                    dependencies,
                    state_keys: if meta.has_named_repeats {
                        super::collect_template_roots::<true, true>(section, &meta.blocks, false)
                            .roots
                    } else {
                        super::collect_template_roots::<false, true>(section, &meta.blocks, false)
                            .roots
                    },
                    state_mode: Some(StateProjectionMode::All as i32),
                    entry: declaration.is_some(),
                },
            });
        }
        emitted
    }

    fn dependencies(&self, section: &TemplateSectionMeta, meta: &TemplateMeta) -> Vec<String> {
        let mut result = HashSet::new();
        let mut visited = HashSet::new();
        let mut pending = vec![section];
        let mut targets = Vec::new();
        while let Some(section) = pending.pop() {
            collect_elements(&section.html, &mut result);
            targets.clear();
            push_ordinary(section, &mut targets);
            for &target in &targets {
                match &self.references[target] {
                    Reference::External(id) => {
                        result.insert(id.clone());
                    }
                    Reference::Inline(_) | Reference::Local(_) if visited.insert(target) => {
                        pending.push(&meta.blocks[target])
                    }
                    Reference::Inline(_) | Reference::Local(_) => {}
                }
            }
        }
        let mut values: Vec<String> = result.into_iter().collect();
        values.sort_unstable();
        values
    }
}

fn push_ordinary(section: &TemplateSectionMeta, pending: &mut Vec<usize>) {
    for (position, (_, index)) in section.conditionals.iter().enumerate() {
        if !section.lazy_conditions.contains(&position) {
            pending.push(*index);
        }
    }
    pending.extend(section.repeats.iter().map(|repeat| repeat.block_index));
}

fn collect_elements(html: &str, components: &mut HashSet<String>) {
    let mut cursor = 0;
    while let Some(offset) = html[cursor..].find('<') {
        cursor += offset;
        let remaining = &html[cursor..];
        let Some(tag) = crate::html_parser::parse_tag(remaining) else {
            cursor += 1;
            continue;
        };
        if !tag.closing && tag.name.contains('-') {
            components.insert(tag.name.to_owned());
        }
        if !tag.closing && !tag.self_closing && super::is_raw_text_or_rcdata_element(tag.name) {
            cursor += super::raw_text_element_end(remaining, tag.name, tag.close + 1);
        } else {
            cursor += tag.close + 1;
        }
    }
}
