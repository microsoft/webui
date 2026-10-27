// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Content-addressed template packs for lazy regions.
//!
//! Each lazy region becomes one immutable JSON file. A template containing a
//! lazy condition carries an `lp` map from region identity to pack URL, so a
//! pack that serializes such a template depends on the URLs of the packs it
//! references; prefetch triggers stay in the condition entries. Packs are
//! rendered in dependency passes: a region renders once every region
//! referenced by its templates has a URL. Build identity is assigned
//! afterwards, over final metadata.

use crate::{HandlerError, Protocol, Result};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;
use webui_protocol::WebUIProtocol;

/// Default public URL prefix for lazy-region packs, relative to the document base.
pub const DEFAULT_PACK_PUBLIC_BASE: &str = "_webui/packs/";

/// Output directory for pack files, relative to the build output directory.
pub const PACK_OUTPUT_DIR: &str = "_webui/packs";

/// One immutable lazy-region pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LazyPackFile {
    /// Content-addressed file name inside [`PACK_OUTPUT_DIR`].
    pub file_name: String,
    /// JSON pack content.
    pub content: String,
}

/// Lazy regions referenced by one template.
type Children = BTreeSet<String>;

/// Render every lazy-region pack and reference it from final template metadata.
///
/// Call after compiling the protocol and before assigning its build identity.
///
/// # Errors
///
/// Returns [`HandlerError::Rendering`] when lazy regions reference each other
/// in a cycle and [`HandlerError::Invariant`] for malformed compiled metadata.
pub fn emit(protocol: &mut WebUIProtocol, public_base: &str) -> Result<Vec<LazyPackFile>> {
    let mut regions: Vec<String> = protocol
        .client_fragments
        .iter()
        .filter(|(_, fragment)| fragment.entry)
        .map(|(id, _)| id.clone())
        .collect();
    if regions.is_empty() {
        return Ok(Vec::new());
    }
    regions.sort_unstable();
    let children = collect_children(protocol)?;
    let initial = Protocol::new(protocol.clone());
    let mut contents = HashMap::with_capacity(regions.len());
    for region in &regions {
        contents.insert(region.as_str(), initial.lazy_pack_templates(region)?);
    }
    drop(initial);

    let mut urls: HashMap<String, String> = HashMap::with_capacity(regions.len());
    let mut finalized = HashSet::new();
    let mut files = Vec::with_capacity(regions.len());
    let mut pending: Vec<&str> = regions.iter().map(String::as_str).collect();
    while !pending.is_empty() {
        let (ready, blocked): (Vec<&str>, Vec<&str>) = std::mem::take(&mut pending)
            .into_iter()
            .partition(|region| {
                contents[region].iter().all(|template| {
                    children
                        .get(template)
                        .is_none_or(|refs| refs.iter().all(|child| urls.contains_key(child)))
                })
            });
        if ready.is_empty() {
            return Err(cyclic_packs(&blocked));
        }
        finalize(protocol, &children, &urls, &mut finalized)?;
        let view = Protocol::new(protocol.clone());
        for region in ready {
            let content = view.render_lazy_pack(region)?;
            let file_name = file_name(&protocol.client_fragments[region].owner, &content);
            urls.insert(region.to_owned(), format!("{public_base}{file_name}"));
            files.push(LazyPackFile { file_name, content });
        }
        pending = blocked;
    }
    finalize(protocol, &children, &urls, &mut finalized)?;
    files.sort_unstable_by(|left, right| left.file_name.cmp(&right.file_name));
    Ok(files)
}

/// Explain why a pack public base cannot be embedded in JSON and HTML.
///
/// Returns `None` for a usable base.
#[must_use]
pub fn public_base_error(base: &str) -> Option<String> {
    let valid = base.ends_with('/')
        && !base.chars().any(|ch| {
            ch.is_whitespace() || ch.is_control() || matches!(ch, '<' | '>' | '"' | '\\')
        });
    (!valid).then(|| invalid_public_base(base))
}

/// Map each template to the lazy regions whose conditions it contains.
fn collect_children(protocol: &WebUIProtocol) -> Result<HashMap<String, Children>> {
    let mut result = HashMap::new();
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
        let children = lazy_children(json).map_err(|error| invalid_metadata(id, &error))?;
        if !children.is_empty() {
            result.insert(id.clone(), children);
        }
    }
    Ok(result)
}

/// Read `[condition, region, slot, trigger]` entries from a template and its blocks.
fn lazy_children(json: &str) -> std::result::Result<Children, String> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|error| error.to_string())?;
    let mut children = Children::new();
    let blocks = value.get("b").and_then(serde_json::Value::as_array);
    let sections = std::iter::once(&value).chain(blocks.into_iter().flatten());
    for section in sections {
        let Some(conditions) = section.get("c").and_then(serde_json::Value::as_array) else {
            continue;
        };
        for entry in conditions {
            let (Some(region), Some(trigger)) = (
                entry.get(1).and_then(serde_json::Value::as_str),
                entry.get(3).and_then(serde_json::Value::as_u64),
            ) else {
                continue;
            };
            if !(1..=4).contains(&trigger) {
                return Err(format!("unknown lazy trigger {trigger} for {region}"));
            }
            children.insert(region.to_owned());
        }
    }
    Ok(children)
}

/// Append `lp` to every template whose referenced regions now all have URLs.
fn finalize(
    protocol: &mut WebUIProtocol,
    children: &HashMap<String, Children>,
    urls: &HashMap<String, String>,
    finalized: &mut HashSet<String>,
) -> Result<()> {
    for (template, refs) in children {
        if finalized.contains(template) || !refs.iter().all(|region| urls.contains_key(region)) {
            continue;
        }
        let entries: BTreeMap<&str, &str> = refs
            .iter()
            .map(|region| (region.as_str(), urls[region].as_str()))
            .collect();
        let encoded = serde_json::to_string(&entries)
            .map_err(|error| HandlerError::Rendering(error.to_string()))?;
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
        json.reserve(encoded.len() + 8);
        json.push_str(",\"lp\":");
        json.push_str(&encoded);
        json.push('}');
        finalized.insert(template.clone());
    }
    Ok(())
}

/// `<owner>.<first 16 hex digits of SHA-256>.json`, safe on every filesystem.
fn file_name(owner: &str, content: &str) -> String {
    let digest = Sha256::digest(content.as_bytes());
    let mut name = String::with_capacity(owner.len() + 22);
    for ch in owner.chars() {
        name.push(
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '.' | '_') {
                ch
            } else {
                '_'
            },
        );
    }
    name.push('.');
    for byte in &digest[..8] {
        let _ = write!(name, "{byte:02x}");
    }
    name.push_str(".json");
    name
}

#[cold]
#[inline(never)]
fn cyclic_packs(regions: &[&str]) -> HandlerError {
    HandlerError::Rendering(format!(
        "lazy regions reference each other in a cycle: {}. Move the shared UI into an ordinary component or remove lazy from one region.",
        regions.join(", ")
    ))
}

#[cold]
#[inline(never)]
fn invalid_metadata(template: &str, error: &str) -> HandlerError {
    HandlerError::Invariant(format!(
        "invalid lazy metadata in {template}: {error}. Rebuild with matching WebUI compiler packages."
    ))
}

#[cold]
#[inline(never)]
fn invalid_public_base(base: &str) -> String {
    format!(
        "invalid pack public base '{base}'. Use a URL or path ending in '/', without spaces, quotes, angle brackets, or backslashes, e.g. '_webui/packs/' or 'https://cdn.example.com/app/packs/'."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lazy_children_read_root_and_block_conditions_only_when_lazy() {
        let json = r#"{"h":"","c":[[0,"o::lazy:@0",[0,0,0],2],[0,1,[0,0,0]]],"b":[{"h":"","c":[[0,"o::lazy:@1",[0,0,0],1],[0,"o::block:3",[0,0,0]]]}],"v":3}"#;
        let children = lazy_children(json).unwrap();
        assert_eq!(
            children.into_iter().collect::<Vec<_>>(),
            ["o::lazy:@0", "o::lazy:@1"]
        );
        assert!(lazy_children(r#"{"c":[[0,"o::lazy:@0",[0,0,0],9]]}"#).is_err());
    }

    #[test]
    fn file_names_are_content_addressed_and_filesystem_safe() {
        let first = file_name("app-shell", "{}");
        assert_eq!(first, file_name("app-shell", "{}"));
        assert_ne!(first, file_name("app-shell", "{ }"));
        assert!(first.starts_with("app-shell.") && first.ends_with(".json"));
        assert_eq!(first.len(), "app-shell.".len() + 16 + ".json".len());
        assert!(file_name("päge:x", "{}").starts_with("p_ge_x."));
    }

    #[test]
    fn public_bases_must_be_safe_directory_prefixes() {
        for base in [
            "_webui/packs/",
            "/static/packs/",
            "https://cdn.example.com/app/",
        ] {
            assert!(public_base_error(base).is_none(), "{base}");
        }
        for base in ["packs", "a b/", "x\"/", "<script>/", "a\\b/", ""] {
            assert!(public_base_error(base).is_some(), "{base}");
        }
    }
}
