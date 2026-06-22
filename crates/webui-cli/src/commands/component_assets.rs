// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use anyhow::{bail, Context, Result};
use rayon::prelude::*;
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use webui::AssetFileNameTemplate;
use webui_handler::route_handler::{render_component_templates, ProtocolIndex};
use webui_protocol::{web_ui_fragment::Fragment, WebUIFragmentRoute, WebUIProtocol};

const ASSET_TYPE: &str = "webui-component-asset";
const ASSET_VERSION: u64 = 1;
const COMPONENT_ASSET_EXT: &str = "webui.js";

/// Summary of emitted component asset files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComponentAssetStats {
    /// Number of requested root component assets emitted.
    pub root_count: usize,
    /// Number of physical files written.
    pub file_count: usize,
}

struct AssetFile {
    name: String,
    content: String,
}

struct ComponentAssetPlan {
    root: String,
    components: Vec<String>,
}

/// Emit static CDN-loadable component assets for the requested root components.
pub fn emit_component_assets(
    protocol: &WebUIProtocol,
    roots: &[String],
    out_dir: &Path,
    file_name_template: &str,
) -> Result<ComponentAssetStats> {
    let plans = plan_component_assets(protocol, roots)?;
    if plans.is_empty() {
        return Ok(ComponentAssetStats {
            root_count: 0,
            file_count: 0,
        });
    }
    let file_name_template =
        AssetFileNameTemplate::try_new(file_name_template.to_string(), "asset_file_name_template")?;

    let files_by_root: Vec<Result<Vec<AssetFile>>> = plans
        .par_iter()
        .map(|plan| render_asset_files(protocol, plan, &file_name_template))
        .collect();

    let mut files = Vec::with_capacity(plans.len());
    for root_files in files_by_root {
        files.extend(root_files?);
    }
    validate_unique_file_names(&files)?;

    files.par_iter().try_for_each(|file| {
        let path = out_dir.join(&file.name);
        fs::write(&path, &file.content)
            .with_context(|| format!("Failed to write component asset {}", path.display()))
    })?;

    Ok(ComponentAssetStats {
        root_count: plans.len(),
        file_count: files.len(),
    })
}

fn plan_component_assets(
    protocol: &WebUIProtocol,
    roots: &[String],
) -> Result<Vec<ComponentAssetPlan>> {
    let roots = validate_roots(protocol, roots)?;
    let mut plans = Vec::with_capacity(roots.len());
    for root in roots {
        plans.push(ComponentAssetPlan {
            components: collect_component_asset_closure(protocol, &root),
            root,
        });
    }
    Ok(plans)
}

fn render_asset_files(
    protocol: &WebUIProtocol,
    plan: &ComponentAssetPlan,
    file_name_template: &AssetFileNameTemplate,
) -> Result<Vec<AssetFile>> {
    let tag_refs: Vec<&str> = plan.components.iter().map(String::as_str).collect();
    let mut index = ProtocolIndex::new(protocol);
    let payload = render_component_templates(protocol, &tag_refs, "", &mut index)
        .with_context(|| format!("Failed to render component asset for <{}>", plan.root))?;
    let mut object = into_object(payload, &plan.root)?;
    let functions = object
        .remove("templateFunctions")
        .unwrap_or_else(|| Value::Object(Map::new()));
    let templates = remove_object(&mut object, "templates", &plan.root)?;
    let template_styles = remove_array(&mut object, "templateStyles", &plan.root)?;

    let mut asset = Map::with_capacity(6);
    asset.insert("type".into(), Value::String(ASSET_TYPE.to_string()));
    asset.insert("version".into(), Value::from(ASSET_VERSION));
    asset.insert(
        "components".into(),
        Value::Array(
            plan.components
                .iter()
                .map(|tag| Value::String(tag.clone()))
                .collect(),
        ),
    );
    asset.insert("templateStyles".into(), Value::Array(template_styles));
    asset.insert("templates".into(), Value::Object(templates));

    let content = build_asset_module(&plan.root, asset, &functions)?;
    let name = file_name_template.resolve(&plan.root, COMPONENT_ASSET_EXT, content.as_bytes());
    Ok(vec![AssetFile { name, content }])
}

fn validate_unique_file_names(files: &[AssetFile]) -> Result<()> {
    let mut names = HashSet::with_capacity(files.len());
    for file in files {
        if !names.insert(file.name.as_str()) {
            bail!(
                "component asset filename collision for '{}'. Adjust --asset-file-name-template to include [name] or another unique component-specific segment.",
                file.name
            );
        }
    }
    Ok(())
}

fn build_asset_module(root: &str, asset: Map<String, Value>, functions: &Value) -> Result<String> {
    let asset_json = serde_json::to_string(&Value::Object(asset))
        .with_context(|| format!("Failed to serialize component asset for <{root}>"))?;
    let mut js = String::with_capacity(asset_json.len() + 64);
    js.push_str("const asset=");
    if has_template_functions(functions)? {
        let Some(prefix) = asset_json.strip_suffix('}') else {
            bail!("component asset for <{root}> was not a serialized JavaScript object");
        };
        js.push_str(prefix);
        js.push_str(",\"templateFunctions\":");
        push_template_functions_object(root, functions, &mut js)?;
        js.push('}');
    } else {
        js.push_str(&asset_json);
    }
    js.push_str(";\nexport default asset;\n");
    Ok(js)
}

fn into_object(payload: Value, root: &str) -> Result<Map<String, Value>> {
    match payload {
        Value::Object(object) => Ok(object),
        _ => bail!("component asset payload for <{root}> was not an object"),
    }
}

fn remove_object(
    object: &mut Map<String, Value>,
    field: &str,
    root: &str,
) -> Result<Map<String, Value>> {
    match object.remove(field) {
        Some(Value::Object(value)) => Ok(value),
        Some(_) => bail!("component asset field '{field}' for <{root}> was not an object"),
        None => Ok(Map::new()),
    }
}

fn remove_array(object: &mut Map<String, Value>, field: &str, root: &str) -> Result<Vec<Value>> {
    match object.remove(field) {
        Some(Value::Array(value)) => Ok(value),
        Some(_) => bail!("component asset field '{field}' for <{root}> was not an array"),
        None => Ok(Vec::new()),
    }
}

fn has_template_functions(functions: &Value) -> Result<bool> {
    let Value::Object(functions) = functions else {
        bail!("component asset field 'templateFunctions' was not an object");
    };
    Ok(!functions.is_empty())
}

fn push_template_functions_object(root: &str, functions: &Value, js: &mut String) -> Result<()> {
    let Value::Object(functions) = functions else {
        bail!("component asset field 'templateFunctions' for <{root}> was not an object");
    };
    let mut tags: Vec<&str> = functions.keys().map(String::as_str).collect();
    tags.sort_unstable();

    js.push('{');
    for (index, tag) in tags.into_iter().enumerate() {
        if index > 0 {
            js.push(',');
        }
        let Some(function_array) = functions.get(tag).and_then(Value::as_str) else {
            bail!("templateFunctions entry for <{tag}> in <{root}> asset was not a string");
        };
        js.push_str(
            &serde_json::to_string(tag)
                .with_context(|| format!("Failed to encode template function tag <{tag}>"))?,
        );
        js.push(':');
        js.push_str(function_array);
    }
    js.push('}');
    Ok(())
}

fn validate_roots(protocol: &WebUIProtocol, roots: &[String]) -> Result<Vec<String>> {
    let mut seen = HashSet::with_capacity(roots.len());
    let mut normalized = Vec::with_capacity(roots.len());
    for raw in roots {
        let tag = raw.trim();
        if tag.is_empty() {
            bail!("--emit-component-assets contains an empty component tag");
        }
        if !is_component_tag_name(tag) {
            bail!(
                "--emit-component-assets component '{tag}' must be a lowercase kebab-case custom element tag"
            );
        }
        if !seen.insert(tag.to_string()) {
            bail!("--emit-component-assets contains duplicate component <{tag}>");
        }
        if !protocol.fragments.contains_key(tag) {
            bail!(
                "--emit-component-assets requested unknown component <{tag}>. Add a discovered {tag}.html component or remove it from the allowlist."
            );
        }
        if !protocol
            .components
            .get(tag)
            .is_some_and(has_template_payload)
        {
            bail!(
                "--emit-component-assets requested <{tag}>, but it has no compiled template metadata. Build with a plugin that emits component templates and ensure the component has a template."
            );
        }
        normalized.push(tag.to_string());
    }
    Ok(normalized)
}

fn is_component_tag_name(tag: &str) -> bool {
    let bytes = tag.as_bytes();
    !bytes.is_empty()
        && bytes.contains(&b'-')
        && bytes[0].is_ascii_lowercase()
        && bytes[bytes.len() - 1].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

fn has_template_payload(component: &webui_protocol::ComponentData) -> bool {
    !component.template_json.is_empty() || !component.template.is_empty()
}

fn collect_component_asset_closure(protocol: &WebUIProtocol, root: &str) -> Vec<String> {
    let mut visited_fragments = HashSet::new();
    let mut components = HashSet::new();
    let mut stack = vec![root.to_string()];

    while let Some(fragment_id) = stack.pop() {
        if fragment_id.is_empty() || !visited_fragments.insert(fragment_id.clone()) {
            continue;
        }

        if protocol
            .components
            .get(&fragment_id)
            .is_some_and(has_template_payload)
        {
            components.insert(fragment_id.clone());
        }

        let Some(fragment_list) = protocol.fragments.get(&fragment_id) else {
            continue;
        };

        for fragment in &fragment_list.fragments {
            match fragment.fragment.as_ref() {
                Some(Fragment::Component(component)) => {
                    stack.push(component.fragment_id.clone());
                }
                Some(Fragment::ForLoop(for_loop)) => {
                    stack.push(for_loop.fragment_id.clone());
                }
                Some(Fragment::IfCond(if_cond)) => {
                    stack.push(if_cond.fragment_id.clone());
                }
                Some(Fragment::Attribute(attr)) if !attr.template.is_empty() => {
                    stack.push(attr.template.clone());
                }
                Some(Fragment::Route(route)) => {
                    push_route_component_ids(route, &mut stack);
                }
                _ => {}
            }
        }
    }

    let mut ordered: Vec<String> = components.into_iter().collect();
    ordered.sort_unstable();
    ordered
}

fn push_route_component_ids(route: &WebUIFragmentRoute, stack: &mut Vec<String>) {
    let mut routes = vec![route];
    while let Some(current) = routes.pop() {
        if !current.fragment_id.is_empty() {
            stack.push(current.fragment_id.clone());
        }
        if !current.pending_component.is_empty() {
            stack.push(current.pending_component.clone());
        }
        if !current.error_component.is_empty() {
            stack.push(current.error_component.clone());
        }
        routes.extend(current.children.iter());
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use webui_protocol::{FragmentList, WebUIFragment};

    fn protocol_with_component(tag: &str) -> WebUIProtocol {
        let mut fragments = std::collections::HashMap::new();
        fragments.insert(
            tag.to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p></p>")],
            },
        );
        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        protocol
            .components
            .entry(tag.to_string())
            .or_default()
            .template_json = r#"{"h":"<p></p>"}"#.to_string();
        protocol
    }

    #[test]
    fn validates_lowercase_kebab_component_tags() {
        assert!(is_component_tag_name("mail-thread"));
        assert!(is_component_tag_name("mail-thread2"));
        assert!(!is_component_tag_name("mail"));
        assert!(!is_component_tag_name("Mail-thread"));
        assert!(!is_component_tag_name("mail_thread"));
        assert!(!is_component_tag_name("mail-thread-"));
    }

    #[test]
    fn validate_roots_rejects_duplicate_tags() {
        let protocol = protocol_with_component("mail-thread");
        let err = validate_roots(
            &protocol,
            &["mail-thread".to_string(), "mail-thread".to_string()],
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("duplicate"));
    }

    #[test]
    fn closure_follows_components_and_all_route_branches() {
        let mut fragments = std::collections::HashMap::new();
        fragments.insert(
            "app-shell".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::component("mail-list"),
                    WebUIFragment::route_from(webui_protocol::WebUiFragmentRoute {
                        path: "compose".to_string(),
                        fragment_id: "compose-page".to_string(),
                        exact: true,
                        children: vec![webui_protocol::WebUiFragmentRoute {
                            path: "preview".to_string(),
                            fragment_id: "compose-preview".to_string(),
                            exact: true,
                            ..Default::default()
                        }],
                        ..Default::default()
                    }),
                ],
            },
        );
        for tag in ["mail-list", "compose-page", "compose-preview"] {
            fragments.insert(
                tag.to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::raw("<p></p>")],
                },
            );
        }
        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        for tag in ["app-shell", "mail-list", "compose-page", "compose-preview"] {
            protocol
                .components
                .entry(tag.to_string())
                .or_default()
                .template_json = r#"{"h":"<p></p>"}"#.to_string();
        }

        let closure = collect_component_asset_closure(&protocol, "app-shell");
        assert_eq!(
            closure,
            vec![
                "app-shell".to_string(),
                "compose-page".to_string(),
                "compose-preview".to_string(),
                "mail-list".to_string(),
            ]
        );
    }
}
