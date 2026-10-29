// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Lazy-region references in compiled template metadata.
//!
//! Reads the condition entries that name lazy regions, and writes the `lp`
//! (pack path) and `lc` (Link stylesheet paths) maps into the templates that
//! hold those conditions once every region they name has been rendered. Both
//! maps are written from the rendered packs themselves, so template metadata
//! always matches the pack report a build returns.

use super::scripts::push_json_string;
use super::{invalid_metadata, LazyPackFile, LazyTrigger, PACK_OUTPUT_DIR};
use crate::Result;
use std::collections::{BTreeSet, HashMap, HashSet};
use webui_protocol::WebUIProtocol;

/// Lazy regions referenced by one template, sorted.
pub(super) type Children = BTreeSet<String>;

/// The packs rendered so far, found by region.
pub(super) struct Rendered {
    packs: Vec<LazyPackFile>,
    regions: HashMap<String, usize>,
}

impl Rendered {
    pub(super) fn with_capacity(capacity: usize) -> Self {
        Self {
            packs: Vec::with_capacity(capacity),
            regions: HashMap::with_capacity(capacity),
        }
    }

    /// Whether `region` has been rendered.
    pub(super) fn contains(&self, region: &str) -> bool {
        self.regions.contains_key(region)
    }

    fn get(&self, region: &str) -> Option<&LazyPackFile> {
        self.regions
            .get(region)
            .and_then(|&index| self.packs.get(index))
    }

    /// Record a rendered pack under its region.
    pub(super) fn push(&mut self, pack: LazyPackFile) {
        self.regions.insert(pack.region.clone(), self.packs.len());
        self.packs.push(pack);
    }

    /// The rendered packs, in rendering order.
    pub(super) fn into_packs(self) -> Vec<LazyPackFile> {
        self.packs
    }
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

/// Append `lp`, and `lc` when a referenced pack registers stylesheets, to
/// every template whose referenced regions have all been rendered.
pub(super) fn finalize(
    protocol: &mut WebUIProtocol,
    children: &HashMap<String, Children>,
    rendered: &Rendered,
    finalized: &mut HashSet<String>,
) -> Result<()> {
    for (template, refs) in children {
        if finalized.contains(template) {
            continue;
        }
        // `refs` is sorted, so both maps are written in region order.
        let Some(packs) = refs
            .iter()
            .map(|region| rendered.get(region))
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
        append_references(json, &packs);
        json.push('}');
        finalized.insert(template.clone());
    }
    Ok(())
}

/// Write `,"lp":{...}` from each pack's path and, when any pack registers
/// stylesheets, `,"lc":{...}` from the [`LazyPackFile::stylesheets`] of only
/// those packs.
fn append_references(json: &mut String, packs: &[&LazyPackFile]) {
    let additional: usize = packs
        .iter()
        .map(|pack| {
            2 * pack.region.len()
                + PACK_OUTPUT_DIR.len()
                + pack.file_name.len()
                + pack
                    .stylesheets
                    .iter()
                    .map(|path| path.len() + 3)
                    .sum::<usize>()
        })
        .sum();
    json.reserve(16 + 8 * packs.len() + additional);
    json.push_str(",\"lp\":{");
    for (position, pack) in packs.iter().enumerate() {
        if position != 0 {
            json.push(',');
        }
        push_json_string(json, &pack.region);
        json.push(':');
        push_json_string(json, &pack.path());
    }
    json.push('}');
    let mut wrote = false;
    for pack in packs {
        if pack.stylesheets.is_empty() {
            continue;
        }
        json.push_str(if wrote { "," } else { ",\"lc\":{" });
        wrote = true;
        push_json_string(json, &pack.region);
        json.push_str(":[");
        for (position, path) in pack.stylesheets.iter().enumerate() {
            if position != 0 {
                json.push(',');
            }
            push_json_string(json, path);
        }
        json.push(']');
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
    fn references_list_every_path_and_only_regions_with_stylesheets() {
        let pack = |region: &str, file_name: &str, stylesheets: &[&str]| LazyPackFile {
            file_name: file_name.to_owned(),
            content: "{}".to_owned(),
            region: region.to_owned(),
            owner: "o".to_owned(),
            trigger: LazyTrigger::Intent,
            components: Vec::new(),
            stylesheets: stylesheets.iter().map(|path| (*path).to_owned()).collect(),
            scripts: Vec::new(),
            brotli_bytes: None,
        };
        let unstyled = pack("o::lazy:@0", "o.0000000000000000.json", &[]);
        let styled = pack(
            "o::lazy:@1",
            "o.1111111111111111.json",
            &["b.css", "q\"uote.css"],
        );
        let mut json = r#"{"v":3"#.to_owned();
        append_references(&mut json, &[&unstyled, &styled]);
        assert_eq!(
            json,
            concat!(
                r#"{"v":3,"lp":{"o::lazy:@0":"_webui/packs/o.0000000000000000.json","#,
                r#""o::lazy:@1":"_webui/packs/o.1111111111111111.json"}"#,
                r#","lc":{"o::lazy:@1":["b.css","q\"uote.css"]}"#,
            )
        );
        let metadata: serde_json::Value = serde_json::from_str(&format!("{json}}}")).unwrap();
        assert_eq!(metadata["lp"]["o::lazy:@1"], styled.path());
        assert_eq!(
            metadata["lc"]["o::lazy:@1"],
            serde_json::json!(styled.stylesheets)
        );

        let mut plain = r#"{"v":3"#.to_owned();
        append_references(&mut plain, &[&unstyled]);
        assert_eq!(
            plain,
            r#"{"v":3,"lp":{"o::lazy:@0":"_webui/packs/o.0000000000000000.json"}"#
        );
    }
}
