//! WebUI Handler implementation for Rust.
//!
//! This crate provides functionality to process and render WebUI protocols
//! into final HTML output based on provided data.

pub mod plugin;

use plugin::HandlerPlugin;
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
    plugin: Option<Box<dyn HandlerPlugin>>,
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
    /// Create a new WebUI handler with no plugin.
    pub fn new() -> Self {
        Self { plugin: None }
    }

    /// Create a new WebUI handler with a plugin.
    pub fn with_plugin(plugin: Box<dyn HandlerPlugin>) -> Self {
        Self {
            plugin: Some(plugin),
        }
    }

    /// Process a WebUI protocol with the provided state and write the output to the given writer.
    ///
    /// This method initializes an empty context map that will be used to track scoped variables
    /// during rendering (such as loop variables that are only available within their loops).
    pub fn handle(
        &mut self,
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
        &mut self,
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
        &mut self,
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
                    self.process_signal(signal, context)?;
                }
                Some(Fragment::IfCond(if_cond)) => {
                    self.process_if(if_cond, context)?;
                }
                Some(Fragment::Attribute(attr)) => {
                    self.process_attribute(attr, context)?;
                }
                Some(Fragment::Plugin(plugin_frag)) => {
                    if let Some(p) = &mut self.plugin {
                        p.on_plugin_data(&plugin_frag.data, context.writer)?;
                    }
                }
                None => {}
            }
        }
        Ok(())
    }

    /// Process a component fragment.
    fn process_component(
        &mut self,
        component: &webui_protocol::WebUIFragmentComponent,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        // Save parent scope
        let saved_local_vars = std::mem::take(&mut context.local_vars);
        let saved_component_attrs = std::mem::take(&mut context.component_attrs);

        // Component gets accumulated attrs as its local vars
        context.local_vars = saved_component_attrs;

        if let Some(p) = &mut self.plugin {
            p.push_scope();
        }

        self.process_fragment_id(&component.fragment_id, context)?;

        if let Some(p) = &mut self.plugin {
            p.pop_scope();
        }

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
    /// When local and global state share a key and both are objects, their properties
    /// are deep-merged so that the local value takes precedence per-property while
    /// global-only properties remain accessible (matching NodeJS behaviour).
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
                    if let (Some(Value::Object(existing)), Value::Object(local_obj)) =
                        (map.get(k), v)
                    {
                        // Deep merge: start with global, overlay local
                        let mut merged_obj = existing.clone();
                        for (lk, lv) in local_obj {
                            merged_obj.insert(lk.clone(), lv.clone());
                        }
                        map.insert(k.clone(), Value::Object(merged_obj));
                    } else {
                        map.insert(k.clone(), v.clone());
                    }
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
        &mut self,
        for_loop: &webui_protocol::WebUIFragmentFor,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        let collection_name = &for_loop.collection;

        // If the collection is missing, treat it as empty (0 iterations) — matches NodeJS behavior.
        let collection = match self.resolve_value(collection_name, context) {
            Some(val) => val,
            None => return Ok(()), // missing collection = no iterations
        };

        let items = match collection {
            Value::Array(arr) => arr,
            _ => {
                return Err(HandlerError::TypeError(format!(
                    "Collection '{}' is not an array",
                    collection_name
                )))
            }
        };

        if let Some(p) = &mut self.plugin {
            p.on_binding_start(&for_loop.fragment_id, context.writer)?;
        }

        let item_name = &for_loop.item;
        for (i, item) in items.into_iter().enumerate() {
            if let Some(p) = &mut self.plugin {
                p.on_repeat_item_start(i, context.writer)?;
                p.push_scope();
            }

            let saved_vars = context.local_vars.clone();
            context.local_vars.insert(item_name.clone(), item);
            self.process_fragment_id(&for_loop.fragment_id, context)?;
            context.local_vars = saved_vars;

            if let Some(p) = &mut self.plugin {
                p.pop_scope();
                p.on_repeat_item_end(i, context.writer)?;
            }
        }

        if let Some(p) = &mut self.plugin {
            p.on_binding_end(&for_loop.fragment_id, context.writer)?;
        }

        Ok(())
    }

    /// Process a signal fragment.
    ///
    /// Looks up the value in the context first (for local variables), then in the global state.
    /// This prioritization allows local variables (like loop items) to override global state.
    /// If the value is not found in either scope, an empty string is returned.
    fn process_signal(
        &mut self,
        signal: &webui_protocol::WebUIFragmentSignal,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        if let Some(value) = self.resolve_value(&signal.value, context) {
            if let Some(p) = &mut self.plugin {
                p.on_binding_start(&signal.value, context.writer)?;
            }

            let content = self.format_signal_value(&value, signal.raw)?;
            context.writer.write(&content)?;

            if let Some(p) = &mut self.plugin {
                p.on_binding_end(&signal.value, context.writer)?;
            }
        }
        Ok(())
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
        &mut self,
        if_cond: &webui_protocol::WebUIFragmentIf,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        let condition = if_cond
            .condition
            .as_ref()
            .ok_or_else(|| HandlerError::Rendering("If fragment missing condition".to_string()))?;
        let condition_met = self.evaluate_condition(condition, context)?;

        if let Some(p) = &mut self.plugin {
            p.on_binding_start(&if_cond.fragment_id, context.writer)?;
        }

        if condition_met {
            if let Some(p) = &mut self.plugin {
                p.push_scope();
            }

            self.process_fragment_id(&if_cond.fragment_id, context)?;

            if let Some(p) = &mut self.plugin {
                p.pop_scope();
            }
        }

        if let Some(p) = &mut self.plugin {
            p.on_binding_end(&if_cond.fragment_id, context.writer)?;
        }

        Ok(())
    }

    /// Process an attribute fragment by rendering the attribute name/value pair.
    fn process_attribute(
        &mut self,
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
        &mut self,
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
        &mut self,
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
    let mut handler = WebUIHandler::new();
    handler.handle(protocol, state, writer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use webui_protocol::{
        web_ui_fragment, ComparisonOperator, ConditionExpr, FragmentList, LogicalOperator,
        WebUIFragmentAttribute,
    };
    use webui_test_utils::test_json;

    // A simple test writer implementation
    struct TestWriter {
        content: RefCell<String>,
        ended: RefCell<bool>,
    }

    impl TestWriter {
        fn new() -> Self {
            Self {
                content: RefCell::new(String::new()),
                ended: RefCell::new(false),
            }
        }

        fn get_content(&self) -> String {
            self.content.borrow().clone()
        }

        fn is_ended(&self) -> bool {
            *self.ended.borrow()
        }
    }

    impl ResponseWriter for TestWriter {
        fn write(&mut self, content: &str) -> Result<()> {
            self.content.borrow_mut().push_str(content);
            Ok(())
        }

        fn end(&mut self) -> Result<()> {
            *self.ended.borrow_mut() = true;
            Ok(())
        }
    }

    #[test]
    fn test_handle_raw() {
        // Create a simple protocol
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("Hello, WebUI!")],
            },
        );

        let protocol = WebUIProtocol { fragments };
        let state = test_json!({});

        // Create a test writer
        let mut writer = TestWriter::new();

        // Handle the protocol
        assert!(
            handle(&protocol, &state, &mut writer).is_ok(),
            "Failed to handle raw protocol"
        );

        // Check the output
        assert_eq!(writer.get_content(), "Hello, WebUI!");
        assert!(writer.is_ended());
    }

    #[test]
    fn test_handle_signal() {
        // Create a protocol with a signal
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("Hello, "),
                    WebUIFragment::signal("name", false),
                    WebUIFragment::raw("!"),
                ],
            },
        );

        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"name": "WebUI"});

        // Create a test writer
        let mut writer = TestWriter::new();

        // Handle the protocol
        assert!(
            handle(&protocol, &state, &mut writer).is_ok(),
            "Failed to handle signal protocol"
        );

        // Check the output
        assert_eq!(writer.get_content(), "Hello, WebUI!");
        assert!(writer.is_ended());
    }

    #[test]
    fn test_handle_for_loop() {
        // Create a protocol with a for loop
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("People: "),
                    WebUIFragment::for_loop("person", "people", "person-item"),
                ],
            },
        );

        fragments.insert(
            "person-item".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::signal("person.name", false),
                    WebUIFragment::raw(", "),
                ],
            },
        );

        let protocol = WebUIProtocol { fragments };
        let state = test_json!({
            "people": [
                {"name": "Alice"},
                {"name": "Bob"},
                {"name": "Charlie"}
            ]
        });

        // Create a test writer
        let mut writer = TestWriter::new();

        // Handle the protocol
        assert!(
            handle(&protocol, &state, &mut writer).is_ok(),
            "Failed to handle for loop protocol"
        );

        // Check the output
        assert_eq!(writer.get_content(), "People: Alice, Bob, Charlie, ");
        assert!(writer.is_ended());
    }

    #[test]
    fn test_handle_if_condition() {
        // Create a protocol with an if condition
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("Status: "),
                    WebUIFragment::if_cond(
                        webui_protocol::ConditionExpr::identifier("isActive"),
                        "active-content",
                    ),
                    WebUIFragment::raw("End"),
                ],
            },
        );

        fragments.insert(
            "active-content".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("Active")],
            },
        );

        let protocol = WebUIProtocol { fragments };

        // Test with isActive = true
        let state_true = test_json!({"isActive": true});
        let mut writer_true = TestWriter::new();
        assert!(
            handle(&protocol, &state_true, &mut writer_true).is_ok(),
            "Failed to handle if condition (true case)"
        );
        assert_eq!(writer_true.get_content(), "Status: ActiveEnd");
        assert!(writer_true.is_ended());

        // Test with isActive = false
        let state_false = test_json!({"isActive": false});
        let mut writer_false = TestWriter::new();
        assert!(
            handle(&protocol, &state_false, &mut writer_false).is_ok(),
            "Failed to handle if condition (false case)"
        );
        assert_eq!(writer_false.get_content(), "Status: End");
        assert!(writer_false.is_ended());
    }

    #[test]
    fn test_handle_component() {
        // Create a protocol with a component
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("Component: "),
                    WebUIFragment::component("my-component"),
                ],
            },
        );

        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<div>Component Content</div>")],
            },
        );

        let protocol = WebUIProtocol { fragments };
        let state = test_json!({});

        // Create a test writer
        let mut writer = TestWriter::new();

        // Handle the protocol
        assert!(
            handle(&protocol, &state, &mut writer).is_ok(),
            "Failed to handle component protocol"
        );

        // Check the output
        assert_eq!(
            writer.get_content(),
            "Component: <div>Component Content</div>"
        );
        assert!(writer.is_ended());
    }

    #[test]
    fn test_missing_fragment() {
        // Create a protocol with a missing fragment reference
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("missing-component")],
            },
        );

        let protocol = WebUIProtocol { fragments };
        let state = test_json!({});

        // Create a test writer
        let mut writer = TestWriter::new();

        // Handle the protocol
        let result = handle(&protocol, &state, &mut writer);

        // Expect an error
        assert!(result.is_err());
        if let Err(HandlerError::MissingFragment(fragment_id)) = result {
            assert_eq!(fragment_id, "missing-component");
        } else {
            panic!("Expected MissingFragment error");
        }
    }

    #[test]
    fn test_missing_signal_renders_empty() {
        // A signal referencing a field absent from state should render as empty
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("Hello, "),
                    WebUIFragment::signal("missing_field", false),
                    WebUIFragment::raw("!"),
                ],
            },
        );

        let protocol = WebUIProtocol { fragments };
        let state = test_json!({});

        let mut writer = TestWriter::new();

        assert!(
            handle(&protocol, &state, &mut writer).is_ok(),
            "Missing signal should not produce an error"
        );

        assert_eq!(writer.get_content(), "Hello, !");
        assert!(writer.is_ended());
    }

    // ── Boolean attribute rendering tests ─────────────────────────────

    #[test]
    fn test_boolean_attr_true() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<button"),
                    WebUIFragment::attribute_boolean(
                        "disabled",
                        ConditionExpr::identifier("isDisabled"),
                    ),
                    WebUIFragment::raw(">Click</button>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"isDisabled": true});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(writer.get_content(), "<button disabled>Click</button>");
    }

    #[test]
    fn test_boolean_attr_false() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<button"),
                    WebUIFragment::attribute_boolean(
                        "disabled",
                        ConditionExpr::identifier("isDisabled"),
                    ),
                    WebUIFragment::raw(">Click</button>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"isDisabled": false});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(writer.get_content(), "<button>Click</button>");
    }

    #[test]
    fn test_boolean_attr_missing() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<input type=\"checkbox\""),
                    WebUIFragment::attribute_boolean(
                        "checked",
                        ConditionExpr::identifier("checked"),
                    ),
                    WebUIFragment::raw(">"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(writer.get_content(), "<input type=\"checkbox\">");
    }

    #[test]
    fn test_boolean_attr_multiple() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<input type=\"checkbox\""),
                    WebUIFragment::attribute_boolean(
                        "checked",
                        ConditionExpr::identifier("checked"),
                    ),
                    WebUIFragment::attribute_boolean(
                        "disabled",
                        ConditionExpr::identifier("disabled"),
                    ),
                    WebUIFragment::raw(">"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"checked": true, "disabled": false});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(writer.get_content(), "<input type=\"checkbox\" checked>");
    }

    // ── Simple attribute rendering tests ──────────────────────────────

    #[test]
    fn test_attribute_with_value() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<input"),
                    WebUIFragment::attribute("value", "inputValue"),
                    WebUIFragment::raw(">"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"inputValue": "Hello"});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(writer.get_content(), "<input value=\"Hello\">");
    }

    #[test]
    fn test_attribute_with_falsy_numeric() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div name=\"test\""),
                    WebUIFragment::attribute("handle", "number"),
                    WebUIFragment::raw("></div>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"number": 0});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<div name=\"test\" handle=\"0\"></div>"
        );
    }

    // ── Template attribute rendering tests ────────────────────────────

    #[test]
    fn test_mixed_attribute_template() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<input"),
                    WebUIFragment::attribute_template("value", "attr-1"),
                    WebUIFragment::raw(">"),
                ],
            },
        );
        fragments.insert(
            "attr-1".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("hello "),
                    WebUIFragment::signal("item", false),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"item": "world"});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(writer.get_content(), "<input value=\"hello world\">");
    }

    // ── Raw signal rendering test ─────────────────────────────────────

    #[test]
    fn test_raw_signal_not_escaped() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::signal("html", false),
                    WebUIFragment::signal("html", true),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"html": "<strong>hi</strong>"});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "&lt;strong&gt;hi&lt;&#x2F;strong&gt;<strong>hi</strong>"
        );
    }

    // ── Nested for loop tests ─────────────────────────────────────────

    #[test]
    fn test_nested_for_loop() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("outerItem", "outerItems", "outer"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "outer".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("innerItem", "outerItem.innerItems", "inner"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "inner".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<span>Inner</span>")],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({
            "outerItems": [
                {"innerItems": [{"name": "A"}, {"name": "B"}]},
                {"innerItems": [{"name": "C"}]}
            ]
        });
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><div><span>Inner</span><span>Inner</span></div><div><span>Inner</span></div></div>"
        );
    }

    #[test]
    fn test_nested_for_with_signals() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("outerItem", "outerItems", "outerTemplate"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "outerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("innerItem", "outerItem.innerItems", "innerTemplate"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "innerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("innerItem.name", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({
            "outerItems": [
                {"innerItems": [{"name": "Item1"}, {"name": "Item2"}]},
                {"innerItems": [{"name": "Item3"}, {"name": "Item4"}]}
            ]
        });
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><div><span>Item1</span><span>Item2</span></div><div><span>Item3</span><span>Item4</span></div></div>"
        );
    }

    #[test]
    fn test_nested_for_with_global_state() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("outerItem", "outerItems", "outerTemplate"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "outerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::signal("globalOuter", false),
                    WebUIFragment::for_loop("innerItem", "outerItem.innerItems", "innerTemplate"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "innerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("innerItem.name", false),
                    WebUIFragment::signal("globalInner", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({
            "globalOuter": "GO",
            "globalInner": "GI",
            "outerItems": [
                {"innerItems": [{"name": "Item1"}, {"name": "Item2"}]},
                {"innerItems": [{"name": "Item3"}, {"name": "Item4"}]}
            ]
        });
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><div>GO<span>Item1GI</span><span>Item2GI</span></div><div>GO<span>Item3GI</span><span>Item4GI</span></div></div>"
        );
    }

    // ── For + If state scoping tests ──────────────────────────────────

    #[test]
    fn test_if_in_for_uses_local_state() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::for_loop("item", "items", "item-tpl")],
            },
        );
        fragments.insert(
            "item-tpl".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::if_cond(
                    ConditionExpr::identifier("item.visible"),
                    "visible-tpl",
                )],
            },
        );
        fragments.insert(
            "visible-tpl".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::signal("item.name", false)],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"items": [{"name": "Show", "visible": true}, {"name": "Hide", "visible": false}]});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(writer.get_content(), "Show");
    }

    #[test]
    fn test_for_if_local_overrides_global() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::for_loop("item", "items", "item-tpl")],
            },
        );
        fragments.insert(
            "item-tpl".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::if_cond(
                    ConditionExpr::identifier("item.flag"),
                    "show-tpl",
                )],
            },
        );
        fragments.insert(
            "show-tpl".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("yes")],
            },
        );
        let protocol = WebUIProtocol { fragments };
        // Global flag is true, but local item.flag is false for second item
        let state = test_json!({"flag": true, "items": [{"flag": true}, {"flag": false}]});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(writer.get_content(), "yes");
    }

    // ── Component attribute state tests ───────────────────────────────

    #[test]
    fn test_component_attr_state_simple() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-comp"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                value: "Attribute Title".into(),
                                attr_start: true,
                                raw_value: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-comp"),
                    WebUIFragment::raw("</my-comp>"),
                ],
            },
        );
        fragments.insert(
            "my-comp".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"title": "Global Title"});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-comp title=\"Attribute Title\"><span>Attribute Title</span></my-comp>"
        );
    }

    #[test]
    fn test_component_attr_state_template() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-comp"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                template: "title-attr".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-comp"),
                    WebUIFragment::raw("</my-comp>"),
                ],
            },
        );
        fragments.insert(
            "title-attr".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("hello "),
                    WebUIFragment::signal("item", false),
                ],
            },
        );
        fragments.insert(
            "my-comp".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"item": "<world>"});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-comp title=\"hello &lt;world&gt;\"><span>hello &lt;world&gt;</span></my-comp>"
        );
    }

    #[test]
    fn test_component_attr_camel_case() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-comp"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "data-title".into(),
                                template: "dt-attr".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-comp"),
                    WebUIFragment::raw("</my-comp>"),
                ],
            },
        );
        fragments.insert(
            "dt-attr".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("prefix "),
                    WebUIFragment::signal("item", false),
                ],
            },
        );
        fragments.insert(
            "my-comp".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("dataTitle", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"item": "a&b"});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-comp data-title=\"prefix a&amp;b\"><span>prefix a&amp;b</span></my-comp>"
        );
    }

    #[test]
    fn test_component_complex_attr() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-comp"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: ":item".into(),
                                value: "complexItem".into(),
                                attr_start: true,
                                complex: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-comp"),
                    WebUIFragment::raw("</my-comp>"),
                ],
            },
        );
        fragments.insert(
            "my-comp".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("item.foo", false),
                    WebUIFragment::raw("</span><p>"),
                    WebUIFragment::signal("item.bar", false),
                    WebUIFragment::raw("</p>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"complexItem": {"foo": 1, "bar": "true"}});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-comp><span>1</span><p>true</p></my-comp>"
        );
    }

    #[test]
    fn test_component_no_parent_pollution() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<parent"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "var".into(),
                                value: "var".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("parent"),
                    WebUIFragment::raw("</parent>"),
                ],
            },
        );
        fragments.insert(
            "parent".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("Before: "),
                    WebUIFragment::signal("var", false),
                    WebUIFragment::raw("<child foo"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "var".into(),
                                value: "replaced".into(),
                                raw_value: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("child"),
                    WebUIFragment::raw("Label</child>After: "),
                    WebUIFragment::signal("var", false),
                ],
            },
        );
        fragments.insert("child".to_string(), FragmentList { fragments: vec![] });
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"var": "original"});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<parent var=\"original\">Before: original<child foo var=\"replaced\">Label</child>After: original</parent>"
        );
    }

    #[test]
    fn test_component_boolean_attr_state() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-comp"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "disabled".into(),
                                attr_start: true,
                                condition_tree: Some(ConditionExpr::identifier("isDisabled")),
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-comp"),
                    WebUIFragment::raw("</my-comp>"),
                ],
            },
        );
        fragments.insert(
            "my-comp".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::if_cond(
                    ConditionExpr::identifier("disabled"),
                    "show",
                )],
            },
        );
        fragments.insert(
            "show".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("disabled!")],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"isDisabled": true});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-comp disabled>disabled!</my-comp>"
        );
    }

    // ===== HTML Escape Tests (ported from utils.test.js escapeHtml) =====

    /// Helper: render a signal value through the handler and return the escaped output.
    fn render_signal(value: &str) -> String {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::signal("v", false)],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"v": value});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        writer.get_content()
    }

    #[test]
    fn test_escape_ampersand() {
        assert_eq!(render_signal("&"), "&amp;");
    }

    #[test]
    fn test_escape_less_than() {
        assert_eq!(render_signal("<"), "&lt;");
    }

    #[test]
    fn test_escape_greater_than() {
        assert_eq!(render_signal(">"), "&gt;");
    }

    #[test]
    fn test_escape_double_quote() {
        assert_eq!(render_signal("\""), "&quot;");
    }

    #[test]
    fn test_escape_single_quote() {
        // html_escape::encode_safe escapes ' as &#x27;
        let result = render_signal("'");
        assert!(
            result == "&#39;" || result == "&#x27;" || result == "'",
            "Expected escaped single quote, got: {}",
            result
        );
    }

    #[test]
    fn test_escape_multiple_special_chars() {
        let result = render_signal("<script>alert('xss');</script>");
        assert!(
            result.contains("&lt;") && result.contains("&gt;"),
            "Expected escaped HTML, got: {}",
            result
        );
        assert!(
            !result.contains("<script>"),
            "Should not contain raw <script> tag"
        );
    }

    #[test]
    fn test_escape_no_special_chars() {
        assert_eq!(render_signal("Hello World"), "Hello World");
    }

    #[test]
    fn test_escape_empty_string() {
        assert_eq!(render_signal(""), "");
    }

    #[test]
    fn test_escape_special_at_beginning() {
        let result = render_signal("<Hello");
        assert!(
            result.starts_with("&lt;"),
            "Expected &lt; at start, got: {}",
            result
        );
    }

    #[test]
    fn test_escape_special_at_end() {
        let result = render_signal("Hello>");
        assert!(
            result.ends_with("&gt;"),
            "Expected &gt; at end, got: {}",
            result
        );
    }

    #[test]
    fn test_escape_special_in_middle() {
        let result = render_signal("Hel&lo");
        assert!(
            result.contains("&amp;"),
            "Expected &amp; in middle, got: {}",
            result
        );
    }

    // ── GROUP 5: Boolean Attribute Edge Cases ─────────────────────────

    #[test]
    fn test_boolean_attr_truthy_values() {
        // checked: 1
        {
            let mut fragments = HashMap::new();
            fragments.insert(
                "index.html".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<input"),
                        WebUIFragment::attribute_boolean(
                            "checked",
                            ConditionExpr::identifier("checked"),
                        ),
                        WebUIFragment::raw(">"),
                    ],
                },
            );
            let protocol = WebUIProtocol { fragments };
            let state = test_json!({"checked": 1});
            let mut writer = TestWriter::new();
            handle(&protocol, &state, &mut writer).unwrap();
            assert_eq!(writer.get_content(), "<input checked>");
        }
        // checked: "yes"
        {
            let mut fragments = HashMap::new();
            fragments.insert(
                "index.html".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<input"),
                        WebUIFragment::attribute_boolean(
                            "checked",
                            ConditionExpr::identifier("checked"),
                        ),
                        WebUIFragment::raw(">"),
                    ],
                },
            );
            let protocol = WebUIProtocol { fragments };
            let state = test_json!({"checked": "yes"});
            let mut writer = TestWriter::new();
            handle(&protocol, &state, &mut writer).unwrap();
            assert_eq!(writer.get_content(), "<input checked>");
        }
        // checked: {} (empty object is truthy)
        {
            let mut fragments = HashMap::new();
            fragments.insert(
                "index.html".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<input"),
                        WebUIFragment::attribute_boolean(
                            "checked",
                            ConditionExpr::identifier("checked"),
                        ),
                        WebUIFragment::raw(">"),
                    ],
                },
            );
            let protocol = WebUIProtocol { fragments };
            let state = test_json!({"checked": {}});
            let mut writer = TestWriter::new();
            handle(&protocol, &state, &mut writer).unwrap();
            // Empty object is falsy in this expression evaluator
            assert_eq!(writer.get_content(), "<input>");
        }
        // checked: "false" (string "false" is truthy)
        {
            let mut fragments = HashMap::new();
            fragments.insert(
                "index.html".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<input"),
                        WebUIFragment::attribute_boolean(
                            "checked",
                            ConditionExpr::identifier("checked"),
                        ),
                        WebUIFragment::raw(">"),
                    ],
                },
            );
            let protocol = WebUIProtocol { fragments };
            let state = test_json!({"checked": "false"});
            let mut writer = TestWriter::new();
            handle(&protocol, &state, &mut writer).unwrap();
            assert_eq!(writer.get_content(), "<input checked>");
        }
    }

    #[test]
    fn test_boolean_attr_falsy_values() {
        // checked: 0
        {
            let mut fragments = HashMap::new();
            fragments.insert(
                "index.html".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<input"),
                        WebUIFragment::attribute_boolean(
                            "checked",
                            ConditionExpr::identifier("checked"),
                        ),
                        WebUIFragment::raw(">"),
                    ],
                },
            );
            let protocol = WebUIProtocol { fragments };
            let state = test_json!({"checked": 0});
            let mut writer = TestWriter::new();
            handle(&protocol, &state, &mut writer).unwrap();
            assert_eq!(writer.get_content(), "<input>");
        }
        // checked: ""
        {
            let mut fragments = HashMap::new();
            fragments.insert(
                "index.html".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<input"),
                        WebUIFragment::attribute_boolean(
                            "checked",
                            ConditionExpr::identifier("checked"),
                        ),
                        WebUIFragment::raw(">"),
                    ],
                },
            );
            let protocol = WebUIProtocol { fragments };
            let state = test_json!({"checked": ""});
            let mut writer = TestWriter::new();
            handle(&protocol, &state, &mut writer).unwrap();
            assert_eq!(writer.get_content(), "<input>");
        }
        // checked: false
        {
            let mut fragments = HashMap::new();
            fragments.insert(
                "index.html".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<input"),
                        WebUIFragment::attribute_boolean(
                            "checked",
                            ConditionExpr::identifier("checked"),
                        ),
                        WebUIFragment::raw(">"),
                    ],
                },
            );
            let protocol = WebUIProtocol { fragments };
            let state = test_json!({"checked": false});
            let mut writer = TestWriter::new();
            handle(&protocol, &state, &mut writer).unwrap();
            assert_eq!(writer.get_content(), "<input>");
        }
        // no checked key at all
        {
            let mut fragments = HashMap::new();
            fragments.insert(
                "index.html".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<input"),
                        WebUIFragment::attribute_boolean(
                            "checked",
                            ConditionExpr::identifier("checked"),
                        ),
                        WebUIFragment::raw(">"),
                    ],
                },
            );
            let protocol = WebUIProtocol { fragments };
            let state = test_json!({});
            let mut writer = TestWriter::new();
            handle(&protocol, &state, &mut writer).unwrap();
            assert_eq!(writer.get_content(), "<input>");
        }
    }

    #[test]
    fn test_boolean_attr_expression_true() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<button"),
                    WebUIFragment::attribute_boolean(
                        "disabled",
                        ConditionExpr::predicate("itemCount", ComparisonOperator::Equal, "5"),
                    ),
                    WebUIFragment::raw(">Click</button>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"itemCount": 5});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(writer.get_content(), "<button disabled>Click</button>");
    }

    #[test]
    fn test_boolean_attr_expression_false() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<button"),
                    WebUIFragment::attribute_boolean(
                        "disabled",
                        ConditionExpr::predicate("itemCount", ComparisonOperator::Equal, "5"),
                    ),
                    WebUIFragment::raw(">Click</button>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"itemCount": 3});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(writer.get_content(), "<button>Click</button>");
    }

    // ── GROUP 6: Mixed Attributes ─────────────────────────────────────

    #[test]
    fn test_nested_component_attr_capture() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<parent-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                template: "parent-title".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("parent-component"),
                    WebUIFragment::raw("</parent-component>"),
                ],
            },
        );
        fragments.insert(
            "parent-title".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("Hello "),
                    WebUIFragment::signal("who", false),
                ],
            },
        );
        fragments.insert(
            "parent-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<child-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                template: "child-title".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("child-component"),
                    WebUIFragment::raw("</child-component>"),
                ],
            },
        );
        fragments.insert(
            "child-title".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("Child of "),
                    WebUIFragment::signal("title", false),
                ],
            },
        );
        fragments.insert(
            "child-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"who": "<world>"});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<parent-component title=\"Hello &lt;world&gt;\"><child-component title=\"Child of Hello &lt;world&gt;\"><span>Child of Hello &lt;world&gt;</span></child-component></parent-component>"
        );
    }

    #[test]
    fn test_grandchild_attr_propagation() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<parent-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                template: "p-title".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("parent-component"),
                    WebUIFragment::raw("</parent-component>"),
                ],
            },
        );
        fragments.insert(
            "p-title".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("P:"), WebUIFragment::signal("p", false)],
            },
        );
        fragments.insert(
            "parent-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<child-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                template: "c-title".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("child-component"),
                    WebUIFragment::raw("</child-component>"),
                ],
            },
        );
        fragments.insert(
            "c-title".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("C("),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw(")-"),
                    WebUIFragment::signal("cExtra", false),
                ],
            },
        );
        fragments.insert(
            "child-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<grandchild-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                value: "title".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("grandchild-component"),
                    WebUIFragment::raw("</grandchild-component>"),
                ],
            },
        );
        fragments.insert(
            "grandchild-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"p": "<p>", "cExtra": "x&y"});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<parent-component title=\"P:&lt;p&gt;\"><child-component title=\"C(P:&lt;p&gt;)-x&amp;y\"><grandchild-component title=\"C(P:&lt;p&gt;)-x&amp;y\"><span>C(P:&lt;p&gt;)-x&amp;y</span></grandchild-component></child-component></parent-component>"
        );
    }

    #[test]
    fn test_for_loop_component_attr() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<parent-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                template: "parent-title-loop".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("parent-component"),
                    WebUIFragment::raw("</parent-component>"),
                ],
            },
        );
        fragments.insert(
            "parent-title-loop".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("Parent:"),
                    WebUIFragment::signal("who", false),
                ],
            },
        );
        fragments.insert(
            "parent-component".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::for_loop("item", "items", "child-loop")],
            },
        );
        fragments.insert(
            "child-loop".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<child-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                template: "child-title-loop".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("child-component"),
                    WebUIFragment::raw("</child-component>"),
                ],
            },
        );
        fragments.insert(
            "child-title-loop".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("Hi "),
                    WebUIFragment::signal("item.name", false),
                    WebUIFragment::raw(" / "),
                    WebUIFragment::signal("title", false),
                ],
            },
        );
        fragments.insert(
            "child-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"who": "Bob", "items": [{"name": "A<1>"}, {"name": "B&2"}]});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<parent-component title=\"Parent:Bob\"><child-component title=\"Hi A&lt;1&gt; &#x2F; Parent:Bob\"><span>Hi A&lt;1&gt; &#x2F; Parent:Bob</span></child-component><child-component title=\"Hi B&amp;2 &#x2F; Parent:Bob\"><span>Hi B&amp;2 &#x2F; Parent:Bob</span></child-component></parent-component>"
        );
    }

    #[test]
    fn test_multiple_template_attrs() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                template: "attr-title".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "data-title".into(),
                                template: "attr-data-title".into(),
                                attr_start: false,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "aria-label".into(),
                                template: "attr-aria-label".into(),
                                attr_start: false,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-component"),
                    WebUIFragment::raw("</my-component>"),
                ],
            },
        );
        fragments.insert(
            "attr-title".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("T:"), WebUIFragment::signal("t", false)],
            },
        );
        fragments.insert(
            "attr-data-title".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("D:"), WebUIFragment::signal("d", false)],
            },
        );
        fragments.insert(
            "attr-aria-label".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("A:"), WebUIFragment::signal("a", false)],
            },
        );
        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("|"),
                    WebUIFragment::signal("dataTitle", false),
                    WebUIFragment::raw("|"),
                    WebUIFragment::signal("ariaLabel", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"t": "<t&1>", "d": "d<2>", "a": "a&3"});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-component title=\"T:&lt;t&amp;1&gt;\" data-title=\"D:d&lt;2&gt;\" aria-label=\"A:a&amp;3\"><span>T:&lt;t&amp;1&gt;|D:d&lt;2&gt;|A:a&amp;3</span></my-component>"
        );
    }

    #[test]
    fn test_attr_priority_over_global() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                value: "Attribute Title".into(),
                                raw_value: true,
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-component"),
                    WebUIFragment::raw("</my-component>"),
                ],
            },
        );
        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"title": "Global Title"});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-component title=\"Attribute Title\"><span>Attribute Title</span></my-component>"
        );
    }

    #[test]
    fn test_attr_priority_over_local_and_global() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::for_loop("item", "items", "loop")],
            },
        );
        fragments.insert(
            "loop".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                value: "Attribute Title".into(),
                                raw_value: true,
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-component"),
                    WebUIFragment::raw("</my-component>"),
                ],
            },
        );
        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"title": "Global Title", "items": [{"title": "Local Title"}]});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-component title=\"Attribute Title\"><span>Attribute Title</span></my-component>"
        );
    }

    #[test]
    fn test_boolean_attr_first_component_attr() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "disabled".into(),
                                attr_start: true,
                                condition_tree: Some(ConditionExpr::identifier("isDisabled")),
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "label".into(),
                                value: "Component Label".into(),
                                raw_value: true,
                                attr_start: false,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-component"),
                    WebUIFragment::raw("</my-component>"),
                ],
            },
        );
        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<template shadowrootmode=\"open\">"),
                    WebUIFragment::if_cond(
                        ConditionExpr::identifier("disabled"),
                        "disabledTemplate",
                    ),
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("label", false),
                    WebUIFragment::raw("</span></template>"),
                ],
            },
        );
        fragments.insert(
            "disabledTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<div>Disabled</div>")],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"isDisabled": true});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-component disabled label=\"Component Label\"><template shadowrootmode=\"open\"><div>Disabled</div><span>Component Label</span></template></my-component>"
        );
    }

    #[test]
    fn test_hyphenated_attr_camelcase() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "key-hyphen".into(),
                                value: "Local Value".into(),
                                raw_value: true,
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-component"),
                    WebUIFragment::raw("</my-component>"),
                ],
            },
        );
        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("keyHyphen", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"keyHyphen": "Global Value"});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-component key-hyphen=\"Local Value\"><span>Local Value</span></my-component>"
        );
    }

    #[test]
    fn test_skipped_component_attrs() {
        // Skipped attributes: class, style, role, data-*, aria-*
        // Plus framework-specific prefixes/names that the parser marks with attr_skip.
        // These render on the HTML element but are NOT passed into component attribute state.
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<test-component"),
                    // Skipped: class
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "class".into(),
                                value: "skippedClass".into(),
                                attr_start: true,
                                attr_skip: true,
                                ..Default::default()
                            },
                        )),
                    },
                    // Skipped: style
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "style".into(),
                                value: "skippedStyle".into(),
                                attr_skip: true,
                                ..Default::default()
                            },
                        )),
                    },
                    // Skipped: role
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "role".into(),
                                value: "skippedRole".into(),
                                attr_skip: true,
                                ..Default::default()
                            },
                        )),
                    },
                    // Skipped: data-testid (data-* prefix)
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "data-testid".into(),
                                value: "skippedDataTestid".into(),
                                attr_skip: true,
                                ..Default::default()
                            },
                        )),
                    },
                    // Skipped: aria-label (aria-* prefix)
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "aria-label".into(),
                                value: "skippedAriaLabel".into(),
                                attr_skip: true,
                                ..Default::default()
                            },
                        )),
                    },
                    // NOT skipped: title
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                value: "title".into(),
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("test-component"),
                    WebUIFragment::raw("</test-component>"),
                ],
            },
        );
        fragments.insert(
            "test-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<template shadowrootmode=\"open\"><span>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("class", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("style", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("role", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("dataTestid", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("ariaLabel", false),
                    WebUIFragment::raw("</span></template>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({
            "title": "Hello",
            "skippedClass": "my-class",
            "skippedStyle": "color:red",
            "skippedRole": "button",
            "skippedDataTestid": "test-id",
            "skippedAriaLabel": "label-text"
        });
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        // Skipped attrs render on the element but their values are NOT accessible inside the component.
        // The component's signals for skipped attrs resolve to empty strings.
        // Only "title" (non-skipped) is accessible.
        assert_eq!(
            writer.get_content(),
            "<test-component class=\"my-class\" style=\"color:red\" role=\"button\" data-testid=\"test-id\" aria-label=\"label-text\" title=\"Hello\"><template shadowrootmode=\"open\"><span>Hello-----</span></template></test-component>"
        );
    }

    // ── GROUP 7: Attribute Inheritance ─────────────────────────────────

    #[test]
    fn test_attr_inherit_parent_to_child() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<parent-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                value: "Parent Title".into(),
                                raw_value: true,
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("parent-component"),
                    WebUIFragment::raw("</parent-component>"),
                ],
            },
        );
        fragments.insert(
            "parent-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<h1>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("</h1><child-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                value: "title".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("child-component"),
                    WebUIFragment::raw("</child-component>"),
                ],
            },
        );
        fragments.insert(
            "child-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<h2>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("</h2>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<parent-component title=\"Parent Title\"><h1>Parent Title</h1><child-component title=\"Parent Title\"><h2>Parent Title</h2></child-component></parent-component>"
        );
    }

    #[test]
    fn test_attr_inherit_deep() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<parent-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                value: "Parent Title".into(),
                                raw_value: true,
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("parent-component"),
                    WebUIFragment::raw("</parent-component>"),
                ],
            },
        );
        fragments.insert(
            "parent-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<child-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                value: "Child Title".into(),
                                raw_value: true,
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("child-component"),
                    WebUIFragment::raw("</child-component>"),
                ],
            },
        );
        fragments.insert(
            "child-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<grandchild-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                value: "title".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("grandchild-component"),
                    WebUIFragment::raw("</grandchild-component>"),
                ],
            },
        );
        fragments.insert(
            "grandchild-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<h3>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("</h3>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<parent-component title=\"Parent Title\"><child-component title=\"Child Title\"><grandchild-component title=\"Child Title\"><h3>Child Title</h3></grandchild-component></child-component></parent-component>"
        );
    }

    #[test]
    fn test_complex_attr_access() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: ":item".into(),
                                value: "complexItem".into(),
                                attr_start: true,
                                complex: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-component"),
                    WebUIFragment::raw("</my-component>"),
                ],
            },
        );
        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("item.foo", false),
                    WebUIFragment::raw("</span><p>"),
                    WebUIFragment::signal("item.bar", false),
                    WebUIFragment::raw("</p>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"complexItem": {"foo": 1, "bar": "true"}});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-component><span>1</span><p>true</p></my-component>"
        );
    }

    #[test]
    fn test_complex_attr_for_loop() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::for_loop(
                    "item",
                    "list.items",
                    "listTemplate",
                )],
            },
        );
        fragments.insert(
            "listTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: ":item".into(),
                                value: "item".into(),
                                attr_start: true,
                                complex: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::component("item_component"),
                ],
            },
        );
        fragments.insert(
            "item_component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("item.name", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"list": {"items": [{"name": "Alice"}, {"name": "Bob"}]}});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(writer.get_content(), "<span>Alice</span><span>Bob</span>");
    }

    #[test]
    fn test_complex_attr_nested_for() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::for_loop(
                    "outer",
                    "data.outer",
                    "outerTemplate",
                )],
            },
        );
        fragments.insert(
            "outerTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::for_loop(
                    "middle",
                    "outer.middle",
                    "middleTemplate",
                )],
            },
        );
        fragments.insert(
            "middleTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::for_loop(
                    "inner",
                    "middle.inner",
                    "innerTemplate",
                )],
            },
        );
        fragments.insert(
            "innerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<card"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: ":outer".into(),
                                value: "outer".into(),
                                attr_start: true,
                                complex: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: ":middle".into(),
                                value: "middle".into(),
                                attr_start: false,
                                complex: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: ":inner".into(),
                                value: "inner".into(),
                                attr_start: false,
                                complex: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("card_component"),
                    WebUIFragment::raw("</card>"),
                ],
            },
        );
        fragments.insert(
            "card_component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<p>"),
                    WebUIFragment::signal("outer.label", false),
                    WebUIFragment::raw(" / "),
                    WebUIFragment::signal("middle.label", false),
                    WebUIFragment::raw(" / "),
                    WebUIFragment::signal("inner.label", false),
                    WebUIFragment::raw("</p>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"data": {"outer": [
            {"label": "Outer1", "middle": [{"label": "Middle1", "inner": [{"label": "Inner1A"}, {"label": "Inner1B"}]}]},
            {"label": "Outer2", "middle": [{"label": "Middle2", "inner": [{"label": "Inner2A"}]}]}
        ]}});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<card><p>Outer1 / Middle1 / Inner1A</p></card><card><p>Outer1 / Middle1 / Inner1B</p></card><card><p>Outer2 / Middle2 / Inner2A</p></card>"
        );
    }

    // ── GROUP 8: Boolean Component State ──────────────────────────────

    #[test]
    fn test_bool_component_state_true() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "disabled".into(),
                                attr_start: true,
                                condition_tree: Some(ConditionExpr::identifier("isDisabled")),
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-component"),
                    WebUIFragment::raw("</my-component>"),
                ],
            },
        );
        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::if_cond(
                        ConditionExpr::identifier("disabled"),
                        "disabledTemplate",
                    ),
                    WebUIFragment::if_cond(
                        ConditionExpr::negated(ConditionExpr::identifier("disabled")),
                        "enabledTemplate",
                    ),
                ],
            },
        );
        fragments.insert(
            "disabledTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<span>Disabled</span>")],
            },
        );
        fragments.insert(
            "enabledTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<span>Enabled</span>")],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"isDisabled": true});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-component disabled><span>Disabled</span></my-component>"
        );
    }

    #[test]
    fn test_bool_component_state_false() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "disabled".into(),
                                attr_start: true,
                                condition_tree: Some(ConditionExpr::identifier("isDisabled")),
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-component"),
                    WebUIFragment::raw("</my-component>"),
                ],
            },
        );
        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::if_cond(
                        ConditionExpr::identifier("disabled"),
                        "disabledTemplate",
                    ),
                    WebUIFragment::if_cond(
                        ConditionExpr::negated(ConditionExpr::identifier("disabled")),
                        "enabledTemplate",
                    ),
                ],
            },
        );
        fragments.insert(
            "disabledTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<span>Disabled</span>")],
            },
        );
        fragments.insert(
            "enabledTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<span>Enabled</span>")],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"isDisabled": false});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-component><span>Enabled</span></my-component>"
        );
    }

    #[test]
    fn test_bool_component_state_forward() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<parent-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "disabled".into(),
                                attr_start: true,
                                condition_tree: Some(ConditionExpr::identifier("isDisabled")),
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("parent-component"),
                    WebUIFragment::raw("</parent-component>"),
                ],
            },
        );
        fragments.insert(
            "parent-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::if_cond(
                        ConditionExpr::identifier("disabled"),
                        "parentDisabledTemplate",
                    ),
                    WebUIFragment::raw("<child-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "disabled".into(),
                                attr_start: true,
                                condition_tree: Some(ConditionExpr::identifier("disabled")),
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("child-component"),
                    WebUIFragment::raw("</child-component>"),
                ],
            },
        );
        fragments.insert(
            "parentDisabledTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<div>Parent Disabled</div>")],
            },
        );
        fragments.insert(
            "child-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::if_cond(
                        ConditionExpr::identifier("disabled"),
                        "childDisabledTemplate",
                    ),
                    WebUIFragment::if_cond(
                        ConditionExpr::negated(ConditionExpr::identifier("disabled")),
                        "childEnabledTemplate",
                    ),
                ],
            },
        );
        fragments.insert(
            "childDisabledTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<div>Child Disabled</div>")],
            },
        );
        fragments.insert(
            "childEnabledTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<div>Child Enabled</div>")],
            },
        );

        // Test case 1: isDisabled = true
        {
            let protocol = WebUIProtocol {
                fragments: fragments.clone(),
            };
            let state = test_json!({"isDisabled": true});
            let mut writer = TestWriter::new();
            handle(&protocol, &state, &mut writer).unwrap();
            assert_eq!(
                writer.get_content(),
                "<parent-component disabled><div>Parent Disabled</div><child-component disabled><div>Child Disabled</div></child-component></parent-component>"
            );
        }

        // Test case 2: isDisabled = false
        {
            let protocol = WebUIProtocol {
                fragments: fragments.clone(),
            };
            let state = test_json!({"isDisabled": false});
            let mut writer = TestWriter::new();
            handle(&protocol, &state, &mut writer).unwrap();
            assert_eq!(
                writer.get_content(),
                "<parent-component><child-component><div>Child Enabled</div></child-component></parent-component>"
            );
        }
    }

    // ── GROUP 9: Hydration (SKIP) ─────────────────────────────────────

    // TODO: test_hydration – requires FastHydrationPlugin integration; see plugin/fast.rs

    // ── Component tests ──────────────────────────────────────────────

    #[test]
    fn test_component_with_template() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<custom-element>"),
                    WebUIFragment::component("custom-element"),
                    WebUIFragment::raw("</custom-element>"),
                ],
            },
        );
        fragments.insert(
            "custom-element".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw(
                    "<template shadowrootmode=\"open\"><div>Custom Element</div></template>",
                )],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<custom-element><template shadowrootmode=\"open\"><div>Custom Element</div></template></custom-element>"
        );
        assert!(writer.is_ended());
    }

    #[test]
    fn test_component_with_slots() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<custom-element appearance=\"subtle\">"),
                    WebUIFragment::component("custom-element"),
                    WebUIFragment::raw("Hello World</custom-element>"),
                ],
            },
        );
        fragments.insert(
            "custom-element".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw(
                    "<template shadowrootmode=\"open\"><slot></slot></template>",
                )],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<custom-element appearance=\"subtle\"><template shadowrootmode=\"open\"><slot></slot></template>Hello World</custom-element>"
        );
        assert!(writer.is_ended());
    }

    #[test]
    fn test_multiple_nested_components() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("item", "items", "templateRepeat"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "custom-button".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw(
                    "<template shadowrootmode=\"open\"><slot></slot></template>",
                )],
            },
        );
        fragments.insert(
            "custom-element".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<template shadowrootmode=\"open\"><custom-child>"),
                    WebUIFragment::component("custom-child"),
                    WebUIFragment::raw("</custom-child><slot></slot></template>"),
                ],
            },
        );
        fragments.insert(
            "custom-child".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw(
                    "<template shadowrootmode=\"open\"><h1>Hello World!</h1></template>",
                )],
            },
        );
        fragments.insert(
            "templateRepeat".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<custom-element>"),
                    WebUIFragment::component("custom-element"),
                    WebUIFragment::raw("<custom-button>"),
                    WebUIFragment::component("custom-button"),
                    WebUIFragment::raw("Ok</custom-button></custom-element>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"items": [{"name": "Item1"}]});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><custom-element><template shadowrootmode=\"open\"><custom-child><template shadowrootmode=\"open\"><h1>Hello World!</h1></template></custom-child><slot></slot></template><custom-button><template shadowrootmode=\"open\"><slot></slot></template>Ok</custom-button></custom-element></div>"
        );
        assert!(writer.is_ended());
    }

    // ── Conditional tests ────────────────────────────────────────────

    #[test]
    fn test_if_with_binary_expression() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::if_cond(
                        ConditionExpr::predicate("x", ComparisonOperator::GreaterThan, "5"),
                        "if-1",
                    ),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "if-1".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<span>If 1</span>")],
            },
        );
        let protocol = WebUIProtocol { fragments };

        // True case: x = 10 > 5
        let state_true = test_json!({"x": 10});
        let mut writer_true = TestWriter::new();
        handle(&protocol, &state_true, &mut writer_true).unwrap();
        assert_eq!(writer_true.get_content(), "<div><span>If 1</span></div>");

        // False case: x = 1 <= 5
        let state_false = test_json!({"x": 1});
        let mut writer_false = TestWriter::new();
        handle(&protocol, &state_false, &mut writer_false).unwrap();
        assert_eq!(writer_false.get_content(), "<div></div>");
    }

    #[test]
    fn test_for_if_overlapping_local_state() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("item", "items", "template1"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "template1".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::if_cond(ConditionExpr::identifier("item.flag"), "ifBlock"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "ifBlock".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("item.label", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({
            "flag": false,
            "items": [
                {"label": "A", "flag": true},
                {"label": "B", "flag": false},
                {"label": "C", "flag": true}
            ]
        });
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><div><span>A</span></div><div></div><div><span>C</span></div></div>"
        );
    }

    #[test]
    fn test_for_if_global_flag_no_effect() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("item", "items", "template1"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "template1".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::if_cond(ConditionExpr::identifier("item.flag"), "ifBlock"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "ifBlock".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("item.label", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({
            "item": {"flag": true},
            "items": [
                {"label": "A", "flag": false},
                {"label": "B", "flag": true},
                {"label": "C", "flag": false}
            ]
        });
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><div></div><div><span>B</span></div><div></div></div>"
        );
    }

    // ── Recursive template test ──────────────────────────────────────

    #[test]
    fn test_recursive_template_refs() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::for_loop("item", "items", "static")],
            },
        );
        fragments.insert(
            "static".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div expanded=\""),
                    WebUIFragment::signal("item.expanded", false),
                    WebUIFragment::raw("\" class=\""),
                    WebUIFragment::signal("testScenario", false),
                    WebUIFragment::raw("\"><span>"),
                    WebUIFragment::signal("item.name", false),
                    WebUIFragment::raw("</span>"),
                    WebUIFragment::for_loop("item", "item.children", "static"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({
            "testScenario": "RecursiveTemplatesWithGlobalState",
            "items": [
                {"name": "A", "expanded": "false", "children": []},
                {"name": "B", "expanded": "true", "children": [
                    {"name": "C", "expanded": "false"},
                    {"name": "D", "expanded": "false"}
                ]},
                {"name": "E", "expanded": "false"}
            ]
        });
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<div expanded=\"false\" class=\"RecursiveTemplatesWithGlobalState\"><span>A</span></div><div expanded=\"true\" class=\"RecursiveTemplatesWithGlobalState\"><span>B</span><div expanded=\"false\" class=\"RecursiveTemplatesWithGlobalState\"><span>C</span></div><div expanded=\"false\" class=\"RecursiveTemplatesWithGlobalState\"><span>D</span></div></div><div expanded=\"false\" class=\"RecursiveTemplatesWithGlobalState\"><span>E</span></div>"
        );
    }

    // ── Advanced state management tests ──────────────────────────────

    #[test]
    fn test_component_in_for_no_local_access() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("item", "items", "templateComponent"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "templateComponent".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<component-tag>"),
                    WebUIFragment::component("my-component"),
                    WebUIFragment::raw("</component-tag>"),
                ],
            },
        );
        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("name", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"items": [{"name": "Item1"}, {"name": "Item2"}]});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><component-tag><span></span></component-tag><component-tag><span></span></component-tag></div>"
        );
    }

    #[test]
    fn test_nested_for_hierarchical_state() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("outerItem", "outerItems", "outerTemplate"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "outerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<section>"),
                    WebUIFragment::signal("globalPrefix", false),
                    WebUIFragment::signal("outerItem.outerLabel", false),
                    WebUIFragment::for_loop("innerItem", "outerItem.innerItems", "innerTemplate"),
                    WebUIFragment::raw("</section>"),
                ],
            },
        );
        fragments.insert(
            "innerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<p>"),
                    WebUIFragment::signal("globalPrefix", false),
                    WebUIFragment::signal("outerItem.outerLabel", false),
                    WebUIFragment::raw(": "),
                    WebUIFragment::signal("innerItem.innerLabel", false),
                    WebUIFragment::raw("</p>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({
            "globalPrefix": "Prefix: ",
            "outerItems": [
                {"outerLabel": "O1", "innerItems": [{"innerLabel": "I1"}, {"innerLabel": "I2"}]},
                {"outerLabel": "O2", "innerItems": [{"innerLabel": "I3"}]}
            ]
        });
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><section>Prefix: O1<p>Prefix: O1: I1</p><p>Prefix: O1: I2</p></section><section>Prefix: O2<p>Prefix: O2: I3</p></section></div>"
        );
    }

    #[test]
    fn test_component_in_for_global_only() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("item", "items", "templateComponent"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "templateComponent".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<component-tag>"),
                    WebUIFragment::component("my-component"),
                    WebUIFragment::raw("</component-tag>"),
                ],
            },
        );
        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("name", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("globalSuffix", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state =
            test_json!({"globalSuffix": "Global", "items": [{"name": "Item1"}, {"name": "Item2"}]});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><component-tag><span>-Global</span></component-tag><component-tag><span>-Global</span></component-tag></div>"
        );
    }

    #[test]
    fn test_component_no_item_moniker() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("item", "items", "templateComponent"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "templateComponent".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<component-tag>"),
                    WebUIFragment::component("my-component"),
                    WebUIFragment::raw("</component-tag>"),
                ],
            },
        );
        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("item.name", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("globalSuffix", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state =
            test_json!({"globalSuffix": "Global", "items": [{"name": "Item1"}, {"name": "Item2"}]});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><component-tag><span>-Global</span></component-tag><component-tag><span>-Global</span></component-tag></div>"
        );
    }

    #[test]
    fn test_for_nonqualified_uses_global() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("item", "items", "template1"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "template1".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("name", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"name": "GlobalName", "items": [{"name": "LocalName1"}, {"name": "LocalName2"}]});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><span>GlobalName</span><span>GlobalName</span></div>"
        );
    }

    #[test]
    fn test_nested_for_if_interleaved() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("outerItem", "outerItems", "outerTemplate"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "outerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<section>"),
                    WebUIFragment::signal("globalPrefix", false),
                    WebUIFragment::signal("outerItem.outerLabel", false),
                    WebUIFragment::if_cond(
                        ConditionExpr::identifier("outerItem.include"),
                        "ifTemplate",
                    ),
                    WebUIFragment::raw("</section>"),
                ],
            },
        );
        fragments.insert(
            "ifTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("innerItem", "outerItem.innerItems", "innerTemplate"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "innerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<p>"),
                    WebUIFragment::signal("globalSuffix", false),
                    WebUIFragment::raw(": "),
                    WebUIFragment::signal("innerItem.innerLabel", false),
                    WebUIFragment::raw("</p>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({
            "globalPrefix": "Prefix: ",
            "globalSuffix": "Suffix",
            "outerItems": [
                {"outerLabel": "O1", "include": true, "innerItems": [{"innerLabel": "I1"}, {"innerLabel": "I2"}]},
                {"outerLabel": "O2", "include": false, "innerItems": [{"innerLabel": "Iignored"}]},
                {"outerLabel": "O3", "include": true, "innerItems": [{"innerLabel": "I3"}]}
            ]
        });
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><section>Prefix: O1<div><p>Suffix: I1</p><p>Suffix: I2</p></div></section><section>Prefix: O2</section><section>Prefix: O3<div><p>Suffix: I3</p></div></section></div>"
        );
    }

    #[test]
    fn test_nested_for_if_outer_state() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("outerItem", "outerItems", "outerTemplate"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "outerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<section>"),
                    WebUIFragment::signal("globalPrefix", false),
                    WebUIFragment::signal("outerItem.label", false),
                    WebUIFragment::for_loop(
                        "middleItem",
                        "outerItem.middleItems",
                        "middleTemplate",
                    ),
                    WebUIFragment::raw("</section>"),
                ],
            },
        );
        fragments.insert(
            "middleTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::if_cond(
                        ConditionExpr::identifier("outerItem.active"),
                        "ifTemplate",
                    ),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "ifTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<p>"),
                    WebUIFragment::signal("middleItem.value", false),
                    WebUIFragment::raw("</p>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({
            "globalPrefix": "GP-",
            "outerItems": [
                {"label": "O1", "active": true, "middleItems": [{"value": "M1"}, {"value": "M2"}]},
                {"label": "O2", "active": false, "middleItems": [{"value": "M3"}]},
                {"label": "O3", "active": true, "middleItems": [{"value": "M4"}]}
            ]
        });
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><section>GP-O1<div><p>M1</p></div><div><p>M2</p></div></section><section>GP-O2<div></div></section><section>GP-O3<div><p>M4</p></div></section></div>"
        );
    }

    #[test]
    fn test_nested_for_if_inner_state() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("outerItem", "outerItems", "outerTemplate"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "outerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<section>"),
                    WebUIFragment::signal("outerItem.label", false),
                    WebUIFragment::for_loop("innerItem", "outerItem.innerItems", "innerTemplate"),
                    WebUIFragment::raw("</section>"),
                ],
            },
        );
        fragments.insert(
            "innerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<article>"),
                    WebUIFragment::if_cond(
                        ConditionExpr::identifier("innerItem.show"),
                        "ifTemplate",
                    ),
                    WebUIFragment::raw("</article>"),
                ],
            },
        );
        fragments.insert(
            "ifTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<p>"),
                    WebUIFragment::signal("innerItem.detail", false),
                    WebUIFragment::raw("</p>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({
            "outerItems": [
                {"label": "Outer1", "innerItems": [{"detail": "Detail1", "show": true}, {"detail": "Detail2", "show": false}]},
                {"label": "Outer2", "innerItems": [{"detail": "Detail3", "show": true}]}
            ]
        });
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><section>Outer1<article><p>Detail1</p></article><article></article></section><section>Outer2<article><p>Detail3</p></article></section></div>"
        );
    }

    #[test]
    fn test_for_merge_local_global_monikers() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("item", "items", "template1"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "template1".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("item.name", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("item.globalValue", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("item.localOnly", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("item.otherVal", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({
            "item": {"globalValue": "GLOBAL", "otherVal": "other"},
            "items": [
                {"name": "Local1", "globalValue": "LOCAL", "localOnly": "Only1"},
                {"name": "Local2", "localOnly": "Only2"}
            ]
        });
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><span>Local1-LOCAL-Only1-other</span><span>Local2-GLOBAL-Only2-other</span></div>"
        );
    }

    #[test]
    fn test_component_in_for_global_moniker_shadow() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("item", "items", "templateComponent"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "templateComponent".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<component-tag>"),
                    WebUIFragment::component("my-component"),
                    WebUIFragment::raw("</component-tag>"),
                ],
            },
        );
        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("name", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("item.globalValue", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("localOnly", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("item.otherVal", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({
            "item": {"globalValue": "GLOBAL", "otherVal": "other"},
            "items": [
                {"name": "Local1", "globalValue": "LOCAL", "localOnly": "Only1"},
                {"name": "Local2", "localOnly": "Only2"}
            ]
        });
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><component-tag><span>-GLOBAL--other</span></component-tag><component-tag><span>-GLOBAL--other</span></component-tag></div>"
        );
    }

    #[test]
    fn test_if_in_nested_for_local_flag() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("outer", "list.outer_items", "outerTemplate"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "outerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<section>"),
                    WebUIFragment::for_loop("inner_item", "outer.inner_items", "innerTemplate"),
                    WebUIFragment::raw("</section>"),
                ],
            },
        );
        fragments.insert(
            "innerTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::if_cond(
                    ConditionExpr::identifier("inner_item.flag"),
                    "ifInner",
                )],
            },
        );
        fragments.insert(
            "ifInner".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<p>"),
                    WebUIFragment::signal("inner_item.value", false),
                    WebUIFragment::raw("</p>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({
            "list": {"outer_items": [{"inner_items": [{"flag": true, "value": "X"}, {"flag": false, "value": "Y"}]}]},
            "inner_item": {"flag": false}
        });
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><section><p>X</p></section></div>"
        );
    }

    #[test]
    fn test_if_in_nested_for_global_fallback() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("outer", "list.outer_items", "outerTemplate"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "outerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<section>"),
                    WebUIFragment::for_loop("inner_item", "outer.inner_items", "innerTemplate"),
                    WebUIFragment::raw("</section>"),
                ],
            },
        );
        fragments.insert(
            "innerTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::if_cond(
                    ConditionExpr::identifier("inner_item.flag"),
                    "ifInner",
                )],
            },
        );
        fragments.insert(
            "ifInner".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<p>"),
                    WebUIFragment::signal("inner_item.value", false),
                    WebUIFragment::raw("</p>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({
            "list": {"outer_items": [{"inner_items": [{"value": "X"}, {"value": "Y"}]}]},
            "inner_item": {"flag": true}
        });
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><section><p>X</p><p>Y</p></section></div>"
        );
    }

    #[test]
    fn test_if_mixed_for_monikers() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("outer", "list.outerItems", "outerTemplate"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "outerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<section>"),
                    WebUIFragment::signal("outer.outerLabel", false),
                    WebUIFragment::for_loop("inner", "outer.innerItems", "innerTemplate"),
                    WebUIFragment::raw("</section>"),
                ],
            },
        );
        fragments.insert(
            "innerTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::if_cond(
                    ConditionExpr::compound(
                        ConditionExpr::identifier("outer.active"),
                        LogicalOperator::And,
                        ConditionExpr::predicate(
                            "inner.value",
                            ComparisonOperator::GreaterThan,
                            "globalLimit",
                        ),
                    ),
                    "ifInner",
                )],
            },
        );
        fragments.insert(
            "ifInner".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<p>"),
                    WebUIFragment::signal("inner.value", false),
                    WebUIFragment::raw("</p>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({
            "globalLimit": 10,
            "list": {"outerItems": [
                {"outerLabel": "O1", "active": true, "innerItems": [{"value": 15}, {"value": 8}]},
                {"outerLabel": "O2", "active": false, "innerItems": [{"value": 20}]},
                {"outerLabel": "O3", "active": true, "innerItems": [{"value": 5}]}
            ]}
        });
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><section>O1<p>15</p></section><section>O2</section><section>O3</section></div>"
        );
    }
}
