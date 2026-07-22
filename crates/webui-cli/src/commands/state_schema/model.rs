// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use serde_json::{Map, Value};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum InferredKind {
    Any,
    Scalar,
    String,
    Boolean,
    Integer,
    Number,
    Object,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PreferredKind {
    String,
    Boolean,
}

#[derive(Default)]
pub(super) struct Node {
    kind: Option<InferredKind>,
    preferred: Option<PreferredKind>,
    required: bool,
    length_kind: Option<InferredKind>,
    length_required: bool,
    children: BTreeMap<String, Node>,
    array_item: Option<Box<Node>>,
}

enum SchemaTask<'a> {
    Visit(&'a Node),
    FinishArray,
    FinishObject(&'a Node),
}

impl Node {
    pub(super) fn root() -> Self {
        Self {
            kind: Some(InferredKind::Object),
            ..Self::default()
        }
    }
}

pub(super) fn add_resolved_path(
    root: &mut Node,
    path: &str,
    kind: InferredKind,
    preferred: Option<PreferredKind>,
    required: bool,
) {
    let mut parts = path.split('.').filter(|part| !part.is_empty()).peekable();
    let mut current = root;
    let mut saw_part = false;
    while let Some(part) = parts.next() {
        if part == "length" && saw_part && parts.peek().is_none() {
            current.length_kind = Some(merge_kind(current.length_kind, kind));
            current.length_required |= required;
            return;
        }
        current = descend_node(current, part, required);
        saw_part = true;
    }
    if saw_part {
        current.kind = Some(merge_kind(current.kind, kind));
        current.preferred = merge_preferred(current.preferred, preferred);
    }
}

pub(super) fn add_array_path(root: &mut Node, path: &str, required: bool) {
    let mut current = root;
    let mut saw_part = false;
    for part in path.split('.').filter(|part| !part.is_empty()) {
        current = descend_node(current, part, required);
        saw_part = true;
    }
    if saw_part {
        current
            .array_item
            .get_or_insert_with(|| Box::new(Node::default()));
    }
}

pub(super) fn resolved_kind(root: &Node, path: &str) -> Option<InferredKind> {
    let mut current = root;
    let mut parts = path.split('.').filter(|part| !part.is_empty()).peekable();
    while let Some(part) = parts.next() {
        if part == "length" && parts.peek().is_none() {
            return current.length_kind;
        }
        let (name, array_depth) = split_array_suffix(part);
        current = current.children.get(name)?;
        for _ in 0..array_depth {
            current = current.array_item.as_deref()?;
        }
    }
    current.kind
}

pub(super) fn node_to_schema(root: &Node) -> Value {
    let mut tasks = vec![SchemaTask::Visit(root)];
    let mut values = Vec::new();
    while let Some(task) = tasks.pop() {
        match task {
            SchemaTask::Visit(node) => {
                if let Some(item) = &node.array_item {
                    tasks.push(SchemaTask::FinishArray);
                    tasks.push(SchemaTask::Visit(item));
                } else if !node.children.is_empty() || node.kind == Some(InferredKind::Object) {
                    tasks.push(SchemaTask::FinishObject(node));
                    for child in node.children.values().rev() {
                        tasks.push(SchemaTask::Visit(child));
                    }
                } else if let Some(kind) = node.length_kind {
                    values.push(length_parent_schema(kind, node.length_required, node.kind));
                } else {
                    values.push(leaf_schema(
                        node.kind.unwrap_or(InferredKind::Any),
                        node.preferred,
                    ));
                }
            }
            SchemaTask::FinishArray => {
                let item = values.pop().unwrap_or_else(any_schema);
                let mut schema = Map::new();
                schema.insert("items".to_string(), item);
                schema.insert("type".to_string(), Value::String("array".to_string()));
                values.push(Value::Object(schema));
            }
            SchemaTask::FinishObject(node) => {
                let child_count = node.children.len();
                let split_at = values.len().saturating_sub(child_count);
                let child_values = values.split_off(split_at);
                let mut properties = Map::new();
                let required_count = node
                    .children
                    .values()
                    .filter(|child| child.required)
                    .count();
                let mut required = Vec::with_capacity(required_count);
                for ((name, child), child_schema) in node.children.iter().zip(child_values) {
                    properties.insert(name.clone(), child_schema);
                    if child.required {
                        required.push(Value::String(name.clone()));
                    }
                }
                if let Some(kind) = node.length_kind {
                    if !properties.contains_key("length") {
                        properties.insert("length".to_string(), leaf_schema(kind, None));
                        if node.length_required {
                            required.push(Value::String("length".to_string()));
                        }
                    }
                }
                required.sort_unstable_by(|left, right| left.as_str().cmp(&right.as_str()));

                let mut schema = Map::new();
                schema.insert("properties".to_string(), Value::Object(properties));
                schema.insert("required".to_string(), Value::Array(required));
                schema.insert("type".to_string(), Value::String("object".to_string()));
                values.push(Value::Object(schema));
            }
        }
    }
    values.pop().unwrap_or_else(|| object_schema(Map::new()))
}

fn descend_node<'a>(current: &'a mut Node, part: &str, required: bool) -> &'a mut Node {
    let (name, array_depth) = split_array_suffix(part);
    if array_depth > 0 {
        current.kind.get_or_insert(InferredKind::Object);
        let array = current.children.entry(name.to_string()).or_default();
        array.required |= required;
        let mut item = array;
        for _ in 0..array_depth {
            item = item
                .array_item
                .get_or_insert_with(|| Box::new(Node::default()));
        }
        return item;
    }

    current.kind.get_or_insert(InferredKind::Object);
    let child = current.children.entry(part.to_string()).or_default();
    child.required |= required;
    child
}

fn split_array_suffix(mut part: &str) -> (&str, usize) {
    let mut depth = 0;
    while let Some(base) = part.strip_suffix("[]") {
        part = base;
        depth += 1;
    }
    (part, depth)
}

fn merge_kind(existing: Option<InferredKind>, incoming: InferredKind) -> InferredKind {
    match (existing, incoming) {
        (Some(InferredKind::Object), _) | (_, InferredKind::Object) => InferredKind::Object,
        (Some(InferredKind::Number), InferredKind::Integer)
        | (Some(InferredKind::Integer), InferredKind::Number) => InferredKind::Number,
        (Some(InferredKind::Any), specific) => specific,
        (Some(specific), InferredKind::Any) => specific,
        (Some(InferredKind::Scalar), specific) => specific,
        (Some(specific), InferredKind::Scalar) => specific,
        (Some(existing), incoming) if existing == incoming => existing,
        (Some(_), _) => InferredKind::Scalar,
        (None, incoming) => incoming,
    }
}

fn merge_preferred(
    existing: Option<PreferredKind>,
    incoming: Option<PreferredKind>,
) -> Option<PreferredKind> {
    match (existing, incoming) {
        (Some(existing), Some(incoming)) if existing == incoming => Some(existing),
        (None, preferred) | (preferred, None) => preferred,
        (Some(_), Some(_)) => None,
    }
}

fn leaf_schema(kind: InferredKind, preferred: Option<PreferredKind>) -> Value {
    let schema = match kind {
        InferredKind::Scalar => scalar_schema(),
        InferredKind::Any => any_schema(),
        InferredKind::String => type_schema("string"),
        InferredKind::Boolean => type_schema("boolean"),
        InferredKind::Integer => type_schema("integer"),
        InferredKind::Number => type_schema("number"),
        InferredKind::Object => object_schema(Map::new()),
    };
    if matches!(kind, InferredKind::Scalar | InferredKind::Any) {
        with_preferred_type(schema, preferred)
    } else {
        schema
    }
}

fn any_schema() -> Value {
    Value::Object(Map::new())
}

fn length_parent_schema(
    kind: InferredKind,
    length_required: bool,
    parent_kind: Option<InferredKind>,
) -> Value {
    match parent_kind {
        Some(InferredKind::Scalar | InferredKind::String) => return type_schema("string"),
        Some(InferredKind::Integer | InferredKind::Number | InferredKind::Boolean) => {
            return Value::Bool(false);
        }
        Some(InferredKind::Object) => return object_schema(Map::new()),
        Some(InferredKind::Any) | None => {}
    }

    let mut array_schema = Map::new();
    array_schema.insert("items".to_string(), any_schema());
    array_schema.insert("type".to_string(), Value::String("array".to_string()));

    let mut object_properties = Map::new();
    object_properties.insert("length".to_string(), leaf_schema(kind, None));
    let mut object_schema = Map::new();
    object_schema.insert("properties".to_string(), Value::Object(object_properties));
    object_schema.insert(
        "required".to_string(),
        if length_required {
            Value::Array(vec![Value::String("length".to_string())])
        } else {
            Value::Array(Vec::new())
        },
    );
    object_schema.insert("type".to_string(), Value::String("object".to_string()));

    let mut schema = Map::new();
    schema.insert(
        "anyOf".to_string(),
        Value::Array(vec![
            type_schema("string"),
            Value::Object(array_schema),
            Value::Object(object_schema),
        ]),
    );
    Value::Object(schema)
}

fn scalar_schema() -> Value {
    let mut schema = Map::new();
    schema.insert(
        "type".to_string(),
        Value::Array(vec![
            Value::String("string".to_string()),
            Value::String("number".to_string()),
            Value::String("boolean".to_string()),
        ]),
    );
    Value::Object(schema)
}

fn type_schema(kind: &str) -> Value {
    let mut schema = Map::new();
    schema.insert("type".to_string(), Value::String(kind.to_string()));
    Value::Object(schema)
}

fn object_schema(properties: Map<String, Value>) -> Value {
    let mut schema = Map::new();
    schema.insert("properties".to_string(), Value::Object(properties));
    schema.insert("required".to_string(), Value::Array(Vec::new()));
    schema.insert("type".to_string(), Value::String("object".to_string()));
    Value::Object(schema)
}

fn with_preferred_type(mut schema: Value, preferred: Option<PreferredKind>) -> Value {
    let Some(preferred) = preferred else {
        return schema;
    };
    let preferred = match preferred {
        PreferredKind::String => "string",
        PreferredKind::Boolean => "boolean",
    };
    if let Some(object) = schema.as_object_mut() {
        let mut webui = Map::new();
        webui.insert(
            "preferredType".to_string(),
            Value::String(preferred.to_string()),
        );
        object.insert("x-webui".to_string(), Value::Object(webui));
    }
    schema
}
