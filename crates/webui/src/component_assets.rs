// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Static component asset rendering for CDN-loadable ESM modules.

use rayon::prelude::*;
use std::collections::HashSet;
use webui_handler::css_module;
use webui_protocol::{web_ui_fragment::Fragment, WebUIFragmentRoute, WebUIProtocol};

use crate::{AssetFileNameTemplate, WebUIError};

const ASSET_TYPE: &str = "webui-component-asset";
const ASSET_VERSION: u64 = 1;
const COMPONENT_ASSET_EXT: &str = "webui.js";

/// A rendered static component asset file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentAssetFile {
    /// Output filename for the ESM asset.
    pub name: String,
    /// JavaScript module content.
    pub content: String,
}

struct ComponentAssetPlan {
    root: String,
    components: Vec<String>,
}

/// Render static CDN-loadable component asset modules for root components.
///
/// Each requested root produces one ESM module. The module contains the root's
/// conservative component dependency closure, template/style metadata, and any
/// WebUI condition closures needed by those templates.
///
/// # Errors
///
/// Returns [`WebUIError`] when the root allowlist is invalid, a requested root
/// has no compiled template metadata, asset filename generation fails, or two
/// component assets resolve to the same filename.
#[must_use = "component asset files must be written or otherwise consumed"]
pub fn render_component_assets(
    protocol: &WebUIProtocol,
    roots: &[String],
    file_name_template: &str,
) -> Result<Vec<ComponentAssetFile>, WebUIError> {
    let plans = plan_component_assets(protocol, roots)?;
    if plans.is_empty() {
        return Ok(Vec::new());
    }

    let file_name_template =
        AssetFileNameTemplate::try_new(file_name_template.to_string(), "asset_file_name_template")
            .map_err(|error| WebUIError::InvalidBuildOptions(error.to_string()))?;

    let rendered: Vec<Result<ComponentAssetFile, WebUIError>> = plans
        .par_iter()
        .map(|plan| render_asset_file(protocol, plan, &file_name_template))
        .collect();

    let mut files = Vec::with_capacity(plans.len());
    for file in rendered {
        files.push(file?);
    }
    validate_unique_asset_file_names(&files)?;
    Ok(files)
}

fn plan_component_assets(
    protocol: &WebUIProtocol,
    roots: &[String],
) -> Result<Vec<ComponentAssetPlan>, WebUIError> {
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

fn render_asset_file(
    protocol: &WebUIProtocol,
    plan: &ComponentAssetPlan,
    file_name_template: &AssetFileNameTemplate,
) -> Result<ComponentAssetFile, WebUIError> {
    let content = build_asset_module(protocol, plan)?;
    let name = file_name_template.resolve(&plan.root, COMPONENT_ASSET_EXT, content.as_bytes());
    Ok(ComponentAssetFile { name, content })
}

fn validate_unique_asset_file_names(files: &[ComponentAssetFile]) -> Result<(), WebUIError> {
    let mut names = HashSet::with_capacity(files.len());
    for file in files {
        if !names.insert(file.name.as_str()) {
            return Err(WebUIError::InvalidBuildOptions(format!(
                "component asset filename collision for '{}'. Adjust --asset-file-name-template to include [name] or another unique component-specific segment.",
                file.name
            )));
        }
    }
    Ok(())
}

fn build_asset_module(
    protocol: &WebUIProtocol,
    plan: &ComponentAssetPlan,
) -> Result<String, WebUIError> {
    let estimated = estimate_asset_module_size(protocol, plan);
    let mut js = String::with_capacity(estimated);
    js.push_str("const asset={\"type\":\"");
    js.push_str(ASSET_TYPE);
    js.push_str("\",\"version\":");
    push_u64(&mut js, ASSET_VERSION);
    js.push_str(",\"components\":[");
    push_string_array(&mut js, &plan.components)?;
    js.push_str("],\"templateStyles\":[");
    push_template_styles(protocol, &plan.components, &mut js)?;
    js.push_str("],\"templates\":{");
    push_templates(protocol, &plan.components, &mut js)?;
    js.push('}');
    if has_template_functions(protocol, &plan.components) {
        js.push_str(",\"templateFunctions\":{");
        push_template_functions(protocol, &plan.root, &plan.components, &mut js)?;
        js.push('}');
    }
    js.push_str("};\nexport default asset;\n");
    Ok(js)
}

fn estimate_asset_module_size(protocol: &WebUIProtocol, plan: &ComponentAssetPlan) -> usize {
    let mut size = 128 + plan.root.len();
    for tag in &plan.components {
        size += tag.len() + 8;
        if let Some(component) = protocol.components.get(tag) {
            size += component.template_json.len();
            size += component.template.len();
            size += component.template_functions.len();
            size += component.css.len();
        }
    }
    size
}

fn push_string_array(out: &mut String, values: &[String]) -> Result<(), WebUIError> {
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        push_json_string(out, value, "component tag")?;
    }
    Ok(())
}

fn push_template_styles(
    protocol: &WebUIProtocol,
    components: &[String],
    out: &mut String,
) -> Result<(), WebUIError> {
    let mut written = 0usize;
    for tag in components {
        let Some(component) = protocol.components.get(tag) else {
            continue;
        };
        if component.css.is_empty() {
            continue;
        }
        if written > 0 {
            out.push(',');
        }
        let tag_html = css_module::build_importmap_tag(tag, &component.css, None);
        push_json_string(out, &tag_html, "component asset templateStyles entry")?;
        written += 1;
    }
    Ok(())
}

fn push_templates(
    protocol: &WebUIProtocol,
    components: &[String],
    out: &mut String,
) -> Result<(), WebUIError> {
    let mut written = 0usize;
    for tag in components {
        let Some(component) = protocol.components.get(tag) else {
            continue;
        };
        if !has_template_payload(component) {
            continue;
        }
        if written > 0 {
            out.push(',');
        }
        push_json_string(out, tag, "component tag")?;
        out.push(':');
        if !component.template_json.is_empty() {
            out.push_str(&component.template_json);
        } else {
            push_json_string(out, &component.template, "component template")?;
        }
        written += 1;
    }
    Ok(())
}

fn has_template_functions(protocol: &WebUIProtocol, components: &[String]) -> bool {
    components.iter().any(|tag| {
        protocol
            .components
            .get(tag)
            .is_some_and(|component| !component.template_functions.is_empty())
    })
}

fn push_template_functions(
    protocol: &WebUIProtocol,
    root: &str,
    components: &[String],
    out: &mut String,
) -> Result<(), WebUIError> {
    let mut written = 0usize;
    for tag in components {
        let Some(component) = protocol.components.get(tag) else {
            continue;
        };
        if component.template_functions.is_empty() {
            continue;
        }
        if written > 0 {
            out.push(',');
        }
        push_json_string(out, tag, "component tag")?;
        out.push(':');
        out.push_str(&component.template_functions);
        written += 1;
    }
    if written == 0 {
        return Err(WebUIError::InvalidBuildOptions(format!(
            "component asset for <{root}> had no template functions to emit"
        )));
    }
    Ok(())
}

fn push_json_string(out: &mut String, value: &str, context: &str) -> Result<(), WebUIError> {
    let encoded = serde_json::to_string(value).map_err(|error| {
        WebUIError::Serialization(format!("Failed to encode {context}: {error}"))
    })?;
    out.push_str(&encoded);
    Ok(())
}

fn push_u64(out: &mut String, value: u64) {
    let mut digits = [0u8; 20];
    let mut n = value;
    let mut i = digits.len();
    if n == 0 {
        out.push('0');
        return;
    }
    while n > 0 {
        i -= 1;
        digits[i] = match n % 10 {
            0 => b'0',
            1 => b'1',
            2 => b'2',
            3 => b'3',
            4 => b'4',
            5 => b'5',
            6 => b'6',
            7 => b'7',
            8 => b'8',
            _ => b'9',
        };
        n /= 10;
    }
    for digit in &digits[i..] {
        out.push(char::from(*digit));
    }
}

fn validate_roots(protocol: &WebUIProtocol, roots: &[String]) -> Result<Vec<String>, WebUIError> {
    let mut seen = HashSet::with_capacity(roots.len());
    let mut normalized = Vec::with_capacity(roots.len());
    for raw in roots {
        let tag = raw.trim();
        if tag.is_empty() {
            return Err(WebUIError::InvalidBuildOptions(
                "--emit-component-assets contains an empty component tag".to_string(),
            ));
        }
        if !is_component_tag_name(tag) {
            return Err(WebUIError::InvalidBuildOptions(format!(
                "--emit-component-assets component '{tag}' must be a lowercase kebab-case custom element tag"
            )));
        }
        if !seen.insert(tag.to_string()) {
            return Err(WebUIError::InvalidBuildOptions(format!(
                "--emit-component-assets contains duplicate component <{tag}>"
            )));
        }
        if !protocol.fragments.contains_key(tag) {
            return Err(WebUIError::InvalidBuildOptions(format!(
                "--emit-component-assets requested unknown component <{tag}>. Add a discovered {tag}.html component or remove it from the allowlist."
            )));
        }
        if !protocol
            .components
            .get(tag)
            .is_some_and(has_template_payload)
        {
            return Err(WebUIError::InvalidBuildOptions(format!(
                "--emit-component-assets requested <{tag}>, but it has no compiled template metadata. Build with a plugin that emits component templates and ensure the component has a template."
            )));
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
    fn render_component_assets_emits_esm_module() {
        let protocol = protocol_with_component("mail-thread");
        let files =
            render_component_assets(&protocol, &["mail-thread".to_string()], "[name].[ext]")
                .unwrap();

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "mail-thread.webui.js");
        assert!(files[0]
            .content
            .contains(r#""type":"webui-component-asset""#));
        assert!(files[0].content.contains("export default asset;"));
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
