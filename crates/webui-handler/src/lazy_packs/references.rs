// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Lazy-region references in compiled template metadata.
//!
//! Reads the condition entries that name lazy regions, and writes the `lp`
//! (pack path) and `lc` (Link stylesheet paths) maps into the templates that
//! hold those conditions once every region they name has been rendered.

use super::scripts::push_json_string;
use super::{invalid_metadata, LazyTrigger};
use crate::Result;
use std::collections::{BTreeSet, HashMap, HashSet};
use webui_protocol::WebUIProtocol;

/// Lazy regions referenced by one template, sorted.
pub(super) type Children = BTreeSet<String>;

/// What templates record about one rendered region.
pub(super) struct Rendered {
    /// `lp` value: the pack's path relative to the asset base.
    pub(super) path: String,
    /// `lc` value: a JSON array of the stylesheet paths the pack registers in
    /// a Link build, or `None` when it registers none.
    pub(super) stylesheets: Option<String>,
}

/// Lazy-region references found in compiled template metadata.
pub(super) struct References {
    /// Regions whose conditions each template contains.
    pub(super) children: HashMap<String, Children>,
    /// Each region's prefetch trigger, from its condition entry.
    pub(super) triggers: HashMap<String, LazyTrigger>,
}

/// Map each template to the lazy regions whose conditions it contains, and
/// each region to its trigger.
pub(super) fn collect_references(protocol: &WebUIProtocol) -> Result<References> {
    let mut references = References {
        children: HashMap::new(),
        triggers: HashMap::new(),
    };
    let templates = protocol
        .components
        .iter()
        .map(|(id, component)| (id, component.template_json.as_str()))
        .chain(
            protocol
                .client_fragments
                .iter()
                .map(|(id, fragment)| (id, fragment.template_json.as_str())),
        );
    for (id, json) in templates {
        if json.is_empty() || !json.contains("::lazy:@") {
            continue;
        }
        let children = lazy_children(json, &mut references.triggers)
            .map_err(|error| invalid_metadata(id, &error))?;
        if !children.is_empty() {
            references.children.insert(id.clone(), children);
        }
    }
    Ok(references)
}

/// Read `[condition, region, slot, trigger]` entries from a template and its blocks.
fn lazy_children(
    json: &str,
    triggers: &mut HashMap<String, LazyTrigger>,
) -> std::result::Result<Children, String> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|error| error.to_string())?;
    let mut children = Children::new();
    let blocks = value.get("b").and_then(serde_json::Value::as_array);
    let sections = std::iter::once(&value).chain(blocks.into_iter().flatten());
    for section in sections {
        let Some(conditions) = section.get("c").and_then(serde_json::Value::as_array) else {
            continue;
        };
        for entry in conditions {
            let (Some(region), Some(code)) = (
                entry.get(1).and_then(serde_json::Value::as_str),
                entry.get(3).and_then(serde_json::Value::as_u64),
            ) else {
                continue;
            };
            let Some(trigger) = LazyTrigger::from_code(code) else {
                return Err(format!("unknown lazy trigger {code} for {region}"));
            };
            triggers.insert(region.to_owned(), trigger);
            children.insert(region.to_owned());
        }
    }
    Ok(children)
}

/// A region's `lc` value: its stylesheet paths as a JSON array, or `None`
/// without any.
pub(super) fn encode_stylesheets(paths: &[&str]) -> Option<String> {
    if paths.is_empty() {
        return None;
    }
    let mut encoded =
        String::with_capacity(2 + paths.iter().map(|path| path.len() + 3).sum::<usize>());
    encoded.push('[');
    for (position, path) in paths.iter().enumerate() {
        if position != 0 {
            encoded.push(',');
        }
        push_json_string(&mut encoded, path);
    }
    encoded.push(']');
    Some(encoded)
}

/// Append `lp`, and `lc` when a referenced pack registers stylesheets, to
/// every template whose referenced regions have all been rendered.
pub(super) fn finalize(
    protocol: &mut WebUIProtocol,
    children: &HashMap<String, Children>,
    rendered: &HashMap<String, Rendered>,
    finalized: &mut HashSet<String>,
) -> Result<()> {
    for (template, refs) in children {
        if finalized.contains(template) {
            continue;
        }
        // `refs` is sorted, so both maps are written in region order.
        let Some(references) = refs
            .iter()
            .map(|region| Some((region.as_str(), rendered.get(region)?)))
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        let json = if let Some(component) = protocol.components.get_mut(template) {
            &mut component.template_json
        } else if let Some(fragment) = protocol.client_fragments.get_mut(template) {
            &mut fragment.template_json
        } else {
            continue;
        };
        if !json.ends_with('}') {
            return Err(invalid_metadata(template, "metadata is not a JSON object"));
        }
        json.pop();
        append_references(json, &references);
        json.push('}');
        finalized.insert(template.clone());
    }
    Ok(())
}

/// Write `,"lp":{...}` and, when any referenced pack registers stylesheets,
/// `,"lc":{...}` listing only those regions.
fn append_references(json: &mut String, references: &[(&str, &Rendered)]) {
    let additional: usize = references
        .iter()
        .map(|(region, pack)| {
            2 * region.len() + pack.path.len() + pack.stylesheets.as_ref().map_or(0, String::len)
        })
        .sum();
    json.reserve(16 + 8 * references.len() + additional);
    json.push_str(",\"lp\":{");
    for (position, (region, pack)) in references.iter().enumerate() {
        if position != 0 {
            json.push(',');
        }
        push_json_string(json, region);
        json.push(':');
        push_json_string(json, &pack.path);
    }
    json.push('}');
    let mut wrote = false;
    for (region, pack) in references {
        let Some(stylesheets) = &pack.stylesheets else {
            continue;
        };
        json.push_str(if wrote { "," } else { ",\"lc\":{" });
        wrote = true;
        push_json_string(json, region);
        json.push(':');
        json.push_str(stylesheets);
    }
    if wrote {
        json.push('}');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lazy_children_read_root_and_block_conditions_only_when_lazy() {
        let json = r#"{"h":"","c":[[0,"o::lazy:@0",[0,0,0],2],[0,1,[0,0,0]]],"b":[{"h":"","c":[[0,"o::lazy:@1",[0,0,0],1],[0,"o::block:3",[0,0,0]]]}],"v":3}"#;
        let mut triggers = HashMap::new();
        let children = lazy_children(json, &mut triggers).unwrap();
        assert_eq!(
            children.into_iter().collect::<Vec<_>>(),
            ["o::lazy:@0", "o::lazy:@1"]
        );
        assert_eq!(triggers["o::lazy:@0"], LazyTrigger::Idle);
        assert_eq!(triggers["o::lazy:@1"], LazyTrigger::Intent);
        assert!(lazy_children(r#"{"c":[[0,"o::lazy:@0",[0,0,0],9]]}"#, &mut triggers).is_err());
    }

    #[test]
    fn stylesheet_lists_are_json_arrays_in_the_given_order() {
        assert_eq!(encode_stylesheets(&[]), None);
        assert_eq!(
            encode_stylesheets(&["b.css", "a.css", "q\"uote.css"]).as_deref(),
            Some(r#"["b.css","a.css","q\"uote.css"]"#)
        );
    }

    #[test]
    fn references_list_every_path_and_only_regions_with_stylesheets() {
        let rendered = [
            Rendered {
                path: "_webui/packs/a.json".to_owned(),
                stylesheets: None,
            },
            Rendered {
                path: "_webui/packs/b.json".to_owned(),
                stylesheets: encode_stylesheets(&["b.css", "chunk.css"]),
            },
        ];
        let mut json = r#"{"v":3"#.to_owned();
        append_references(
            &mut json,
            &[("o::lazy:@0", &rendered[0]), ("o::lazy:@1", &rendered[1])],
        );
        assert_eq!(
            json,
            concat!(
                r#"{"v":3,"lp":{"o::lazy:@0":"_webui/packs/a.json","o::lazy:@1":"_webui/packs/b.json"}"#,
                r#","lc":{"o::lazy:@1":["b.css","chunk.css"]}"#,
            )
        );

        let mut unstyled = r#"{"v":3"#.to_owned();
        append_references(&mut unstyled, &[("o::lazy:@0", &rendered[0])]);
        assert_eq!(
            unstyled,
            r#"{"v":3,"lp":{"o::lazy:@0":"_webui/packs/a.json"}"#
        );
    }
}
