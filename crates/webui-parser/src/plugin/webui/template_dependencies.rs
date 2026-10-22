// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::HashSet;

use webui_protocol::{web_ui_fragment::Fragment, WebUIFragmentRecords};

use crate::plugin::ComponentTemplateArtifact;
use crate::{ParserError, Result};

pub(super) fn attach(
    templates: &mut [ComponentTemplateArtifact],
    fragments: &WebUIFragmentRecords,
) -> Result<()> {
    let mut known = HashSet::with_capacity(templates.len());
    let mut payloads = Vec::with_capacity(templates.len());
    let mut lazy_payloads = Vec::new();
    for template in templates {
        if template.template_json.is_empty() {
            continue;
        }
        let tag = template.tag_name.as_str();
        known.insert(tag);
        payloads.push((tag, &mut template.template_json));
        for fragment in &mut template.lazy_templates {
            known.insert(fragment.id.as_str());
            lazy_payloads.push(&mut fragment.data);
        }
    }
    if payloads.is_empty() {
        return Ok(());
    }

    let mut scratch = DependencyScratch::default();
    for (tag, json) in payloads {
        scratch.collect(fragments, &known, tag)?;
        append_dependencies(tag, json, &scratch.dependencies)?;
    }
    for data in lazy_payloads {
        data.dependencies.retain(|id| known.contains(id.as_str()));
        if !data.dependencies.is_empty() {
            data.template_json.truncate(data.template_json.len() - 1);
            data.template_json.push_str(",\"d\":[");
            for (position, id) in data.dependencies.iter().enumerate() {
                if position != 0 {
                    data.template_json.push(',');
                }
                super::emit_js_string(id, &mut data.template_json);
            }
            data.template_json.push_str("]}");
        }
    }
    Ok(())
}

#[derive(Default)]
struct DependencyScratch<'a> {
    visited: HashSet<&'a str>,
    seen_components: HashSet<&'a str>,
    pending: Vec<&'a str>,
    dependencies: Vec<&'a str>,
}

impl<'a> DependencyScratch<'a> {
    fn collect(
        &mut self,
        fragments: &'a WebUIFragmentRecords,
        known: &HashSet<&str>,
        root: &'a str,
    ) -> Result<()> {
        self.visited.clear();
        self.seen_components.clear();
        self.pending.clear();
        self.dependencies.clear();
        self.pending.push(root);

        while let Some(id) = self.pending.pop() {
            if !self.visited.insert(id) {
                continue;
            }
            let list = fragments
                .get(id)
                .ok_or_else(|| missing_fragment(root, id))?;
            for fragment in &list.fragments {
                match fragment.fragment.as_ref() {
                    Some(Fragment::Component(component)) => {
                        let child = component.fragment_id.as_str();
                        if known.contains(child) {
                            if self.seen_components.insert(child) {
                                self.dependencies.push(child);
                            }
                        } else {
                            self.pending.push(child);
                        }
                    }
                    Some(Fragment::ForLoop(repeat)) => {
                        self.pending.push(repeat.fragment_id.as_str());
                    }
                    Some(Fragment::IfCond(condition)) if !condition.lazy => {
                        self.pending.push(condition.fragment_id.as_str());
                    }
                    Some(Fragment::Attribute(attribute)) if !attribute.template.is_empty() => {
                        self.pending.push(attribute.template.as_str());
                    }
                    // Route activation owns its destination independently.
                    _ => {}
                }
            }
        }

        self.dependencies.sort_unstable();
        Ok(())
    }
}

fn append_dependencies(tag: &str, json: &mut String, dependencies: &[&str]) -> Result<()> {
    if !json.starts_with('{') || !json.ends_with('}') {
        return Err(invalid_payload(tag));
    }
    json.truncate(json.len() - 1);
    if !dependencies.is_empty() {
        json.push_str(",\"d\":[");
        for (index, dependency) in dependencies.iter().enumerate() {
            if index != 0 {
                json.push(',');
            }
            super::emit_js_string(dependency, json);
        }
        json.push(']');
    }
    json.push('}');
    Ok(())
}

#[cold]
#[inline(never)]
fn missing_fragment(component: &str, fragment: &str) -> ParserError {
    ParserError::ClientTemplateMetadata {
        component: component.into(),
        reason: format!("missing compiled fragment '{fragment}'").into_boxed_str(),
    }
}

#[cold]
#[inline(never)]
fn invalid_payload(component: &str) -> ParserError {
    ParserError::ClientTemplateMetadata {
        component: component.into(),
        reason: "the WebUI plugin did not produce an object metadata payload".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use webui_protocol::{FragmentList, WebUIFragment};

    #[test]
    fn named_fragment_cycles_are_walked_once() {
        let fragments = WebUIFragmentRecords::from([
            (
                "x-root".into(),
                FragmentList {
                    fragments: vec![WebUIFragment::for_loop("item", "items", "body")],
                    ..Default::default()
                },
            ),
            (
                "body".into(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::component("x-child"),
                        WebUIFragment::for_loop("item", "item.children", "body"),
                    ],
                    ..Default::default()
                },
            ),
        ]);
        let mut scratch = DependencyScratch::default();
        scratch
            .collect(&fragments, &HashSet::from(["x-root", "x-child"]), "x-root")
            .expect("finite graph walk");
        assert_eq!(scratch.dependencies, ["x-child"]);
    }

    #[test]
    fn missing_fragments_return_a_typed_metadata_error() {
        let mut scratch = DependencyScratch::default();
        assert!(matches!(
            scratch.collect(&WebUIFragmentRecords::new(), &HashSet::new(), "x-root"),
            Err(ParserError::ClientTemplateMetadata { .. })
        ));
    }

    #[test]
    fn invalid_plugin_payloads_are_not_repaired_silently() {
        let mut json = "invalid".to_string();
        assert!(matches!(
            append_dependencies("x-root", &mut json, &[]),
            Err(ParserError::ClientTemplateMetadata { .. })
        ));
        assert_eq!(json, "invalid");
    }
}
