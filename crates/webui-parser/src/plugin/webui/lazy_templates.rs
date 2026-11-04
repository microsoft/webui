// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::{
    emit_js_string, emit_json_template_section, ConditionFunctionEmitter, TemplateMeta,
    TemplateSectionMeta,
};
use crate::plugin::LazyTemplateArtifact;
use std::collections::{HashMap, HashSet};
use webui_protocol::{ClientFragmentData, StateProjectionMode};

pub(super) enum Reference {
    Inline(usize),
    Local(usize),
    External(String),
}

pub(super) struct Plan {
    pub(super) references: Vec<Reference>,
    bundles: Vec<Vec<usize>>,
    emitted: Vec<bool>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Owner {
    Unassigned,
    Eager,
    Region(usize),
    Shared,
}

impl Plan {
    pub(super) fn new(owner: &str, meta: &TemplateMeta) -> Option<Self> {
        if meta.lazy.is_empty() && meta.blocks.len() < 2 {
            return None;
        }
        let owners = block_owners(meta);
        let mut next = 0;
        let mut bundles = vec![Vec::new(); meta.lazy.len()];
        let mut references = Vec::with_capacity(meta.blocks.len());
        let mut emitted = vec![true; meta.blocks.len()];
        let mut leaves: HashMap<Owner, HashMap<String, usize>> = HashMap::new();
        for (index, owner_kind) in owners.into_iter().enumerate() {
            let key = matches!(owner_kind, Owner::Eager | Owner::Region(_))
                .then(|| Self::leaf_key(&meta.blocks[index]))
                .flatten();
            if let Some(key) = &key {
                if let Some(&local) = leaves
                    .get(&owner_kind)
                    .and_then(|leaves| leaves.get(key.as_str()))
                {
                    emitted[index] = false;
                    references.push(match owner_kind {
                        Owner::Eager => Reference::Inline(local),
                        _ => Reference::Local(local),
                    });
                    continue;
                }
            }
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
            if let Some(key) = key {
                let local = match references[index] {
                    Reference::Inline(local) | Reference::Local(local) => local,
                    Reference::External(_) => continue,
                };
                leaves.entry(owner_kind).or_default().insert(key, local);
            }
        }
        for declaration in &meta.lazy {
            references[declaration.block_index] = Reference::External(declaration.id.clone());
        }
        if meta.lazy.is_empty() && emitted.iter().all(|emit| *emit) {
            return None;
        }
        Some(Self {
            references,
            bundles,
            emitted,
        })
    }

    pub(super) fn is_eager(&self, index: usize) -> bool {
        self.emitted[index] && matches!(self.references[index], Reference::Inline(_))
    }

    fn leaf_key(section: &TemplateSectionMeta) -> Option<String> {
        if !section.conditionals.is_empty()
            || !section.repeats.is_empty()
            || section
                .attr_bindings
                .iter()
                .any(|binding| matches!(binding, super::CompiledAttrBinding::Boolean { .. }))
        {
            return None;
        }
        let mut key = String::with_capacity(section.html.len() + 128);
        emit_json_template_section(
            section,
            &mut key,
            &mut ConditionFunctionEmitter::new(0),
            None,
        );
        Some(key)
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

fn block_owners(meta: &TemplateMeta) -> Vec<Owner> {
    if meta.lazy.is_empty() {
        return vec![Owner::Eager; meta.blocks.len()];
    }
    let mut owners = vec![Owner::Unassigned; meta.blocks.len()];
    let mut pending = Vec::new();
    push_ordinary(&meta.root, &mut pending);
    while let Some(index) = pending.pop() {
        if owners[index] == Owner::Eager {
            continue;
        }
        owners[index] = Owner::Eager;
        push_ordinary(&meta.blocks[index], &mut pending);
    }
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
    owners
}

fn push_ordinary(section: &TemplateSectionMeta, pending: &mut Vec<usize>) {
    for (position, (_, index)) in section.conditionals.iter().enumerate() {
        if !section.is_lazy_condition(position) {
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

#[cfg(test)]
mod tests {
    use super::{Plan, Reference};
    use crate::plugin::webui::{compile_to_metadata, generate_compiled_template};

    #[test]
    fn repeated_leaf_data_uses_one_existing_local_block_index() {
        let source = r#"<select><for each="option in first"><option value="{{option.value}}" @click="{pick(option.value)}" w-ref="{choice}">{{option.label}}</option></for></select><select><for each="option in second"><option value="{{option.value}}" @click="{pick(option.value)}" w-ref="{choice}">{{option.label}}</option></for></select>"#;
        let meta = compile_to_metadata("test-options", source.into(), Vec::new()).unwrap();
        assert_eq!(meta.blocks.len(), 2);
        let plan = Plan::new("test-options", &meta).unwrap();
        assert!(plan.is_eager(0));
        assert!(!plan.is_eager(1));
        assert!(matches!(plan.references[0], Reference::Inline(0)));
        assert!(matches!(plan.references[1], Reference::Inline(0)));
        let json = generate_compiled_template("test-options", source).unwrap();
        assert_eq!(json.matches(r#""h":"<option></option>""#).count(), 1);
        assert!(json.contains(r#"["first","option",0,"#));
        assert!(json.contains(r#"["second","option",0,"#));
    }

    #[test]
    fn equal_html_does_not_merge_different_bindings_or_scope_names() {
        for source in [
            r#"<for each="item in first"><i>{{item.name}}</i></for><for each="item in second"><i>{{item.title}}</i></for>"#,
            r#"<for each="item in first"><i>{{item.name}}</i></for><for each="other in second"><i>{{other.name}}</i></for>"#,
            r#"<if condition="first"><input ?disabled="{{active}}"></if><if condition="second"><input ?disabled="{{other}}"></if>"#,
        ] {
            let meta = compile_to_metadata("test-distinct", source.into(), Vec::new()).unwrap();
            assert!(Plan::new("test-distinct", &meta).is_none());
        }
    }

    #[test]
    fn sharing_stays_inside_each_lazy_delivery_owner() {
        let body = r#"<select><for each="option in first"><option>{{option.label}}</option></for></select><select><for each="option in second"><option>{{option.label}}</option></for></select>"#;
        let source = format!(
            r#"<if condition="firstOpen" lazy>{body}</if><if condition="secondOpen" lazy>{body}</if>"#
        );
        let meta = compile_to_metadata("test-regions", source.as_str().into(), Vec::new()).unwrap();
        let plan = Plan::new("test-regions", &meta).unwrap();
        let fragments = plan.emit("test-regions", &meta);
        assert_eq!(fragments.len(), 2);
        assert_ne!(fragments[0].id, fragments[1].id);
        for fragment in fragments {
            assert_eq!(
                fragment
                    .data
                    .template_json
                    .matches(r#""h":"<option></option>""#)
                    .count(),
                1
            );
            assert!(fragment
                .data
                .template_json
                .contains(r#"["first","option",-1,"#));
            assert!(fragment
                .data
                .template_json
                .contains(r#"["second","option",-1,"#));
        }
    }
}
