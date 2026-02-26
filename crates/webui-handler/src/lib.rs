//! WebUI Handler implementation for Rust.
//!
//! This crate provides functionality to process and render WebUI protocols
//! into final HTML output based on provided data.

use serde_json::Value;
use std::collections::HashMap;
use thiserror::Error;
use webui_expressions::{evaluate, ExpressionError};
use webui_protocol::{web_ui_fragment::Fragment, WebUIFragment, WebUIProtocol};
use webui_state::find_value_by_dotted_path;

/// Error types for the WebUI handler.
#[derive(Debug, Error)]
pub enum HandlerError {
    #[error("Rendering error: {0}")]
    Rendering(String),

    #[error("Missing fragment: {0}")]
    MissingFragment(String),

    #[error("Missing data field: {0}")]
    MissingData(String),

    #[error("Type error: {0}")]
    TypeError(String),

    #[error("Protocol error: {0}")]
    Protocol(#[from] webui_protocol::ProtocolError),

    #[error("Evaluation error: {0}")]
    Evaluation(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Writer error: {0}")]
    Writer(String),
}

pub type Result<T> = std::result::Result<T, HandlerError>;

/// Interface for writing rendered output
pub trait ResponseWriter {
    /// Write content to the output
    fn write(&mut self, content: &str) -> Result<()>;

    /// Finalize the output
    fn end(&mut self) -> Result<()>;
}

/// The main WebUI handler that processes protocols and renders them.
pub struct WebUIHandler {
    // Configuration options could go here
}

/// Context object for processing WebUI fragments
struct WebUIProcessContext<'a> {
    protocol: &'a WebUIProtocol,
    state: &'a Value,
    #[allow(dead_code)]
    depth: usize,
    writer: &'a mut dyn ResponseWriter,
    // Add local variables map to store context-specific variables (like loop items)
    local_vars: HashMap<String, Value>,
    /// Accumulates component attribute values between attrStart and the component fragment.
    component_attrs: HashMap<String, Value>,
}

/// Convert hyphenated name to camelCase (e.g., "data-title" → "dataTitle").
fn convert_hyphen_to_camel_case(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    let mut capitalize_next = false;
    for ch in name.chars() {
        if ch == '-' {
            capitalize_next = true;
        } else if capitalize_next {
            result.extend(ch.to_uppercase());
            capitalize_next = false;
        } else {
            result.push(ch);
        }
    }
    result
}

/// Get the component attribute name, stripping `:` prefix and converting to camelCase.
fn component_attr_name(name: &str) -> String {
    let stripped = name.strip_prefix(':').unwrap_or(name);
    if stripped.contains('-') {
        convert_hyphen_to_camel_case(stripped)
    } else {
        stripped.to_string()
    }
}

impl WebUIHandler {
    /// Create a new WebUI handler.
    pub fn new() -> Self {
        Self {}
    }

    /// Process a WebUI protocol with the provided state and write the output to the given writer.
    ///
    /// This method initializes an empty context map that will be used to track scoped variables
    /// during rendering (such as loop variables that are only available within their loops).
    pub fn handle(
        &self,
        protocol: &WebUIProtocol,
        state: &Value,
        writer: &mut dyn ResponseWriter,
    ) -> Result<()> {
        // Start with the main fragment (typically "index.html")
        let main_fragment_id = "index.html";
        if !protocol.fragments.contains_key(main_fragment_id) {
            return Err(HandlerError::MissingFragment(main_fragment_id.to_string()));
        }

        // Process the main fragment with an empty initial context
        let mut context = WebUIProcessContext {
            protocol,
            state,
            depth: 0,
            writer,
            local_vars: HashMap::new(),
            component_attrs: HashMap::new(),
        };
        self.process_fragment_id(main_fragment_id, &mut context)?;

        // Finalize the output
        writer.end()?;

        Ok(())
    }

    /// Process a fragment by its ID.
    ///
    /// The `context` parameter contains scope-local variables that are accessible during rendering,
    /// such as loop iteration variables. This is separate from the global `state`.
    fn process_fragment_id(
        &self,
        fragment_id: &str,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        if let Some(fragment_list) = context.protocol.fragments.get(fragment_id) {
            self.process_fragment(&fragment_list.fragments, context)
        } else {
            Err(HandlerError::MissingFragment(fragment_id.to_string()))
        }
    }

    /// Process a vector of fragments.
    ///
    /// The `context` maintains scope-specific variables that can be accessed by fragments
    /// during rendering, while `state` contains the global application state.
    fn process_fragment(
        &self,
        fragments: &[WebUIFragment],
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        for item in fragments {
            match item.fragment.as_ref() {
                Some(Fragment::Raw(raw)) => {
                    context.writer.write(&raw.value)?;
                }
                Some(Fragment::Component(component)) => {
                    self.process_component(component, context)?;
                }
                Some(Fragment::ForLoop(for_loop)) => {
                    self.process_for_loop(for_loop, context)?;
                }
                Some(Fragment::Signal(signal)) => {
                    let content = self.process_signal(signal, context)?;
                    context.writer.write(&content)?;
                }
                Some(Fragment::IfCond(if_cond)) => {
                    self.process_if(if_cond, context)?;
                }
                Some(Fragment::Attribute(attr)) => {
                    self.process_attribute(attr, context)?;
                }
                None => {}
            }
        }
        Ok(())
    }

    /// Process a component fragment.
    fn process_component(
        &self,
        component: &webui_protocol::WebUIFragmentComponent,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        // Save parent scope
        let saved_local_vars = std::mem::take(&mut context.local_vars);
        let saved_component_attrs = std::mem::take(&mut context.component_attrs);

        // Component gets accumulated attrs as its local vars
        context.local_vars = saved_component_attrs;

        self.process_fragment_id(&component.fragment_id, context)?;

        // Restore parent scope
        context.local_vars = saved_local_vars;
        context.component_attrs = HashMap::new();

        Ok(())
    }

    /// Resolve a dotted path value, checking local variables first, then global state.
    fn resolve_value(&self, path: &str, context: &WebUIProcessContext) -> Option<Value> {
        // Check local vars first
        if let Some(first_part) = path.split('.').next() {
            if let Some(local_value) = context.local_vars.get(first_part) {
                if !path.contains('.') {
                    return Some(local_value.clone());
                }
                let remaining = &path[first_part.len() + 1..];
                if let Some(v) = find_value_by_dotted_path(remaining, local_value) {
                    return Some(v);
                }
            }
        }
        // Fall back to global state
        find_value_by_dotted_path(path, context.state)
    }

    /// Evaluate a condition expression, merging local variables into state.
    /// Returns false if the condition references a missing value.
    fn evaluate_condition(
        &self,
        condition: &webui_protocol::ConditionExpr,
        context: &WebUIProcessContext,
    ) -> Result<bool> {
        let merged_state = if context.local_vars.is_empty() {
            context.state.clone()
        } else {
            let mut merged = context.state.clone();
            if let Value::Object(map) = &mut merged {
                for (k, v) in &context.local_vars {
                    map.insert(k.clone(), v.clone());
                }
            }
            merged
        };
        match evaluate(condition, &merged_state) {
            Ok(result) => Ok(result),
            Err(ExpressionError::MissingValue(_)) => Ok(false),
            Err(e) => Err(HandlerError::Evaluation(e.to_string())),
        }
    }

    /// Process a for loop fragment.
    ///
    /// Creates a new context for each iteration that includes the current loop item.
    /// This allows nested templates to access both the loop variable and any parent context.
    /// Example: `for item in items` makes "item" available in the loop body.
    fn process_for_loop(
        &self,
        for_loop: &webui_protocol::WebUIFragmentFor,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        let collection_name = &for_loop.collection;

        let collection = self
            .resolve_value(collection_name, context)
            .ok_or_else(|| HandlerError::MissingData(collection_name.to_string()))?;

        let items = match collection {
            Value::Array(arr) => arr,
            _ => {
                return Err(HandlerError::TypeError(format!(
                    "Collection '{}' is not an array",
                    collection_name
                )))
            }
        };

        let item_name = &for_loop.item;
        for item in items {
            let saved_vars = context.local_vars.clone();
            context.local_vars.insert(item_name.clone(), item);
            self.process_fragment_id(&for_loop.fragment_id, context)?;
            context.local_vars = saved_vars;
        }

        Ok(())
    }

    /// Process a signal fragment.
    ///
    /// Looks up the value in the context first (for local variables), then in the global state.
    /// This prioritization allows local variables (like loop items) to override global state.
    /// If the value is not found in either scope, an empty string is returned.
    fn process_signal(
        &self,
        signal: &webui_protocol::WebUIFragmentSignal,
        context: &WebUIProcessContext,
    ) -> Result<String> {
        if let Some(value) = self.resolve_value(&signal.value, context) {
            return self.format_signal_value(&value, signal.raw);
        }
        Ok(String::new())
    }

    /// Helper function to format a signal value based on the raw flag
    fn format_signal_value(&self, value: &Value, raw: bool) -> Result<String> {
        let result = if raw {
            // Raw HTML content
            match value {
                Value::String(s) => s.clone(),
                _ => value.to_string(),
            }
        } else {
            // Escaped HTML content
            match value {
                Value::String(s) => html_escape::encode_safe(s).to_string(),
                _ => html_escape::encode_safe(&value.to_string()).to_string(),
            }
        };
        Ok(result)
    }

    /// Process an if condition fragment.
    fn process_if(
        &self,
        if_cond: &webui_protocol::WebUIFragmentIf,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        let condition = if_cond
            .condition
            .as_ref()
            .ok_or_else(|| HandlerError::Rendering("If fragment missing condition".to_string()))?;
        let condition_met = self.evaluate_condition(condition, context)?;

        if condition_met {
            self.process_fragment_id(&if_cond.fragment_id, context)?;
        }

        Ok(())
    }

    /// Process an attribute fragment by rendering the attribute name/value pair.
    fn process_attribute(
        &self,
        attr: &webui_protocol::WebUIFragmentAttribute,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        // Initialize component attribute accumulator on attrStart
        if attr.attr_start {
            context.component_attrs = HashMap::new();
        }

        // Boolean attribute with condition tree
        if let Some(condition) = &attr.condition_tree {
            let condition_met = self.evaluate_condition(condition, context)?;

            if !attr.attr_skip {
                let name = component_attr_name(&attr.name);
                context
                    .component_attrs
                    .insert(name, Value::Bool(condition_met));
            }

            if condition_met {
                context.writer.write(&format!(" {}", attr.name))?;
            }
            return Ok(());
        }

        // Template attribute (mixed static + dynamic)
        if !attr.template.is_empty() {
            let raw_value = self.render_template_attr_value(&attr.template, context)?;
            let escaped = html_escape::encode_safe(&raw_value);
            context
                .writer
                .write(&format!(" {}=\"{}\"", attr.name, escaped))?;

            if !attr.attr_skip {
                let name = component_attr_name(&attr.name);
                context
                    .component_attrs
                    .insert(name, Value::String(raw_value));
            }
            return Ok(());
        }

        // Simple attribute
        if !attr.value.is_empty() {
            if attr.raw_value {
                // Static attribute — value is the literal string
                context
                    .writer
                    .write(&format!(" {}=\"{}\"", attr.name, attr.value))?;
                if !attr.attr_skip {
                    let name = component_attr_name(&attr.name);
                    context
                        .component_attrs
                        .insert(name, Value::String(attr.value.clone()));
                }
            } else if attr.complex {
                // Complex attribute — resolve value, don't render to HTML, store as state
                if let Some(value) = self.resolve_value(&attr.value, context) {
                    if !attr.attr_skip {
                        let stripped = attr.name.strip_prefix(':').unwrap_or(&attr.name);
                        let name = component_attr_name(stripped);
                        context.component_attrs.insert(name, value);
                    }
                }
            } else {
                // Dynamic attribute — resolve and render
                if let Some(value) = self.resolve_value(&attr.value, context) {
                    let formatted = match &value {
                        Value::String(s) => html_escape::encode_safe(s).to_string(),
                        Value::Number(n) => n.to_string(),
                        Value::Bool(b) => b.to_string(),
                        Value::Null => String::new(),
                        _ => value.to_string(),
                    };
                    context
                        .writer
                        .write(&format!(" {}=\"{}\"", attr.name, formatted))?;

                    if !attr.attr_skip {
                        let name = component_attr_name(&attr.name);
                        context.component_attrs.insert(name, value);
                    }
                }
            }
        }

        Ok(())
    }

    /// Render a template attribute's fragments into a raw (unescaped) string.
    fn render_template_attr_value(
        &self,
        template_id: &str,
        context: &WebUIProcessContext,
    ) -> Result<String> {
        let fragments = context
            .protocol
            .fragments
            .get(template_id)
            .ok_or_else(|| HandlerError::MissingFragment(template_id.to_string()))?;
        let mut raw_value = String::new();
        for frag in &fragments.fragments {
            match frag.fragment.as_ref() {
                Some(Fragment::Raw(raw)) => raw_value.push_str(&raw.value),
                Some(Fragment::Signal(signal)) => {
                    if let Some(value) = self.resolve_value(&signal.value, context) {
                        match &value {
                            Value::String(s) => raw_value.push_str(s),
                            _ => raw_value.push_str(&value.to_string()),
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(raw_value)
    }

    /// Render the UI based on the protocol and state
    pub fn render(
        &self,
        protocol: &WebUIProtocol,
        state: &Value,
        writer: &mut dyn ResponseWriter,
    ) -> Result<()> {
        let mut context = WebUIProcessContext {
            protocol,
            state,
            depth: 0,
            writer,
            local_vars: HashMap::new(),
            component_attrs: HashMap::new(),
        };

        self.process_fragment_id("index.html", &mut context)
    }
}

impl Default for WebUIHandler {
    fn default() -> Self {
        Self::new()
    }
}

/// Process a WebUI protocol with the provided state and write the output to the given writer.
/// This is the main entry point for the WebUI handler.
pub fn handle(
    protocol: &WebUIProtocol,
    state: &Value,
    writer: &mut dyn ResponseWriter,
) -> Result<()> {
    let handler = WebUIHandler::new();
    handler.handle(protocol, state, writer)
}

#[cfg(test)]
mod tests;
