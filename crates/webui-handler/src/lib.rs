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
mod tests {
    use super::*;
    use std::cell::RefCell;
    use webui_protocol::{web_ui_fragment, ConditionExpr, FragmentList, WebUIFragmentAttribute};
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
                fragments: vec![WebUIFragment::for_loop("item", "items", "item-tpl")],
            },
        );
        fragments.insert(
            "item-tpl".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<p>"),
                    WebUIFragment::signal("item.name", false),
                    WebUIFragment::raw("</p>"),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"items": [{"name": "Alice"}, {"name": "Bob"}]});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(writer.get_content(), "<p>Alice</p><p>Bob</p>");
    }

    #[test]
    fn test_nested_for_with_global_state() {
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
                fragments: vec![
                    WebUIFragment::signal("item.name", false),
                    WebUIFragment::raw(" - "),
                    WebUIFragment::signal("globalTitle", false),
                ],
            },
        );
        let protocol = WebUIProtocol { fragments };
        let state = test_json!({"items": [{"name": "A"}, {"name": "B"}], "globalTitle": "Title"});
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &mut writer).unwrap();
        assert_eq!(writer.get_content(), "A - TitleB - Title");
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
}
