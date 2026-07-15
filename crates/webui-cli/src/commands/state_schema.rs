// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

mod inference;
mod model;
mod scope;
#[cfg(test)]
mod tests;

use anyhow::{bail, Context, Result};
use clap::Args;
use expand_tilde::expand_tilde;
use inference::SchemaInference;
use serde::ser::{SerializeMap, Serializer};
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::path::PathBuf;
use webui_handler::route_matcher;
use webui_protocol::web_ui_fragment;
use webui_protocol::{WebUIFragmentRoute, WebUIProtocol};

use crate::utils::output;

const JSON_SCHEMA_DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";
pub(super) const DEFAULT_SCHEMA_TITLE: &str = "WebUIState";

#[derive(Args)]
pub struct SchemaArgs {
    /// Path to a protocol.bin file
    pub file: PathBuf,

    /// Entry fragment to analyze
    #[arg(long, default_value = "index.html")]
    pub entry: String,

    /// Schema title
    #[arg(long, default_value = DEFAULT_SCHEMA_TITLE)]
    pub title: String,
}

struct RouteChain<'a> {
    path: String,
    components: Vec<&'a str>,
}

struct PendingRoute<'a> {
    route: &'a WebUIFragmentRoute,
    parent_path: String,
    components: Vec<&'a str>,
}

pub(super) struct SchemaDocument<'a>(&'a Value);

impl<'a> SchemaDocument<'a> {
    pub(super) fn new(schema: &'a Value) -> Self {
        Self(schema)
    }
}

impl Serialize for SchemaDocument<'_> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let Value::Object(schema) = self.0 else {
            return self.0.serialize(serializer);
        };

        let mut map = serializer.serialize_map(Some(schema.len()))?;
        if let Some(dialect) = schema.get("$schema") {
            map.serialize_entry("$schema", dialect)?;
        }
        for (name, value) in schema {
            if name != "$schema" {
                map.serialize_entry(name, value)?;
            }
        }
        map.end()
    }
}

pub(super) fn schema_to_pretty_json(schema: &Value) -> serde_json::Result<String> {
    serde_json::to_string_pretty(&SchemaDocument::new(schema))
}

pub fn execute(args: &SchemaArgs) -> Result<()> {
    run(args).inspect_err(|error| {
        output::error(error);
        eprintln!();
    })
}

fn run(args: &SchemaArgs) -> Result<()> {
    let input_file = expand_tilde(&args.file)
        .with_context(|| format!("Failed to expand input path: {}", args.file.display()))?
        .into_owned();

    let protocol = WebUIProtocol::from_protobuf_file(&input_file)
        .with_context(|| format!("Failed to read protocol {}", input_file.display()))?;
    let schema = generate_schema(&protocol, &args.entry, &args.title)
        .with_context(|| format!("Failed to generate schema for entry {}", args.entry))?;
    let output = schema_to_pretty_json(&schema)?;
    println!("{output}");
    Ok(())
}

pub(super) fn generate_schema(protocol: &WebUIProtocol, entry: &str, title: &str) -> Result<Value> {
    if !protocol.fragments.contains_key(entry) {
        bail!("Entry fragment '{entry}' was not found in the protocol");
    }

    let routes = collect_route_chains(protocol, entry)?;
    if routes.is_empty() {
        let mut inference = SchemaInference::new(protocol);
        inference.infer_entry(entry)?;
        return Ok(with_schema_metadata(inference.into_schema(), title));
    }

    generate_route_bundle(protocol, entry, title, &routes)
}

fn generate_route_bundle(
    protocol: &WebUIProtocol,
    entry: &str,
    title: &str,
    routes: &[RouteChain<'_>],
) -> Result<Value> {
    let mut definitions = Map::new();
    let mut route_refs = Map::new();
    let mut alternatives = Vec::with_capacity(routes.len());
    let mut definition_keys = BTreeSet::new();

    for route in routes {
        let key = route_definition_key(&route.path);
        if !definition_keys.insert(key.clone()) {
            bail!(
                "Route path '{}' collides with another generated schema definition",
                route.path
            );
        }

        let mut inference = SchemaInference::new(protocol);
        inference.infer_entry(entry)?;
        for component in &route.components {
            inference.infer_root_component(component)?;
        }

        let route_title = route_schema_title(title, &route.path);
        let schema = with_title(inference.into_schema(), &route_title);
        let schema_ref = definition_ref(&key);
        definitions.insert(key, schema);
        route_refs.insert(route.path.clone(), Value::String(schema_ref.clone()));
        alternatives.push(ref_schema(schema_ref));
    }

    let mut schema = Map::new();
    schema.insert(
        "$schema".to_string(),
        Value::String(JSON_SCHEMA_DIALECT.to_string()),
    );
    schema.insert("$defs".to_string(), Value::Object(definitions));
    schema.insert("anyOf".to_string(), Value::Array(alternatives));
    schema.insert("title".to_string(), Value::String(title.to_string()));
    schema.insert("x-webui-routes".to_string(), Value::Object(route_refs));
    Ok(Value::Object(schema))
}

fn collect_route_chains<'a>(
    protocol: &'a WebUIProtocol,
    entry: &str,
) -> Result<Vec<RouteChain<'a>>> {
    let root_routes = collect_root_routes(protocol, entry)?;

    if root_routes.is_empty() {
        return Ok(Vec::new());
    }

    let mut pending = Vec::with_capacity(root_routes.len());
    for route in root_routes.into_iter().rev() {
        pending.push(PendingRoute {
            route,
            parent_path: "/".to_string(),
            components: Vec::new(),
        });
    }

    let mut chains = Vec::new();
    while let Some(current) = pending.pop() {
        let path = normalize_route_path(&route_matcher::resolve_route_path(
            &current.route.path,
            &current.parent_path,
        ));
        let mut components = current.components;
        let component_was_visited = components
            .iter()
            .any(|component| *component == current.route.fragment_id);
        components.push(current.route.fragment_id.as_str());

        let mut child_routes = Vec::with_capacity(current.route.children.len());
        child_routes.extend(&current.route.children);
        if !component_was_visited {
            child_routes.extend(collect_root_routes(protocol, &current.route.fragment_id)?);
        }

        let has_zero_segment_child = child_routes
            .iter()
            .any(|child| child_matches_parent_path(&path, &child.path));
        if child_routes.is_empty() || !has_zero_segment_child {
            chains.push(RouteChain {
                path: path.clone(),
                components: components.clone(),
            });
        }

        for child in child_routes.into_iter().rev() {
            pending.push(PendingRoute {
                route: child,
                parent_path: path.clone(),
                components: components.clone(),
            });
        }
    }

    chains.sort_unstable_by(|left, right| left.path.cmp(&right.path));
    for pair in chains.windows(2) {
        if pair[0].path == pair[1].path {
            bail!(
                "Multiple route chains resolve to the schema path '{}'",
                pair[0].path
            );
        }
    }
    Ok(chains)
}

fn collect_root_routes<'a>(
    protocol: &'a WebUIProtocol,
    entry: &str,
) -> Result<Vec<&'a WebUIFragmentRoute>> {
    let mut routes = Vec::new();
    let mut visited = BTreeSet::new();
    let mut pending = vec![entry];

    while let Some(fragment_id) = pending.pop() {
        if !visited.insert(fragment_id) {
            continue;
        }
        let fragment_list = protocol
            .fragments
            .get(fragment_id)
            .with_context(|| format!("Protocol fragment '{fragment_id}' was not found"))?;
        for fragment in &fragment_list.fragments {
            match fragment.fragment.as_ref() {
                Some(web_ui_fragment::Fragment::Component(component)) => {
                    pending.push(component.fragment_id.as_str());
                }
                Some(web_ui_fragment::Fragment::ForLoop(for_loop)) => {
                    pending.push(for_loop.fragment_id.as_str());
                }
                Some(web_ui_fragment::Fragment::IfCond(if_cond)) => {
                    pending.push(if_cond.fragment_id.as_str());
                }
                Some(web_ui_fragment::Fragment::Route(route)) => routes.push(route),
                _ => {}
            }
        }
    }
    Ok(routes)
}

fn child_matches_parent_path(parent: &str, child: &str) -> bool {
    let resolved = normalize_route_path(&route_matcher::resolve_route_path(child, parent));
    if resolved == parent {
        return true;
    }
    if child.starts_with('/') {
        return false;
    }

    let relative = child.strip_prefix("./").unwrap_or(child);
    relative
        .split('/')
        .filter(|segment| !segment.is_empty())
        .all(|segment| {
            segment.starts_with('*') || (segment.starts_with(':') && segment.ends_with('?'))
        })
}

fn normalize_route_path(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        let mut normalized = String::with_capacity(trimmed.len() + 1);
        normalized.push('/');
        normalized.push_str(trimmed);
        normalized
    }
}

fn route_definition_key(path: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut key = String::with_capacity(path.len() + "route.".len());
    key.push_str("route");
    for segment in path.trim_matches('/').split('/') {
        if segment.is_empty() {
            continue;
        }
        key.push('.');
        for byte in segment.bytes() {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b':' | b'?' | b'*') {
                key.push(char::from(byte));
            } else {
                key.push('%');
                key.push(char::from(HEX[usize::from(byte >> 4)]));
                key.push(char::from(HEX[usize::from(byte & 0x0f)]));
            }
        }
    }
    key
}

fn definition_ref(key: &str) -> String {
    let mut schema_ref = String::with_capacity("#/$defs/".len() + key.len());
    schema_ref.push_str("#/$defs/");
    schema_ref.push_str(key);
    schema_ref
}

fn route_schema_title(title: &str, path: &str) -> String {
    let mut route_title = String::with_capacity(title.len() + path.len() + 3);
    route_title.push_str(title);
    route_title.push_str(" (");
    route_title.push_str(path);
    route_title.push(')');
    route_title
}

fn ref_schema(schema_ref: String) -> Value {
    let mut schema = Map::new();
    schema.insert("$ref".to_string(), Value::String(schema_ref));
    Value::Object(schema)
}

fn with_schema_metadata(schema: Value, title: &str) -> Value {
    let mut schema = with_title(schema, title);
    if let Some(object) = schema.as_object_mut() {
        object.insert(
            "$schema".to_string(),
            Value::String(JSON_SCHEMA_DIALECT.to_string()),
        );
    }
    schema
}

fn with_title(mut schema: Value, title: &str) -> Value {
    if let Some(object) = schema.as_object_mut() {
        object.insert("title".to_string(), Value::String(title.to_string()));
    }
    schema
}
