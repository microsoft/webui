//! WebUI Handler implementation for Rust.
//!
//! This crate provides functionality to process and render WebUI protocols
//! into final HTML output based on provided data.

use serde_json::Value;
use std::collections::HashMap;
use thiserror::Error;
use webui_expressions::evaluate;
use webui_protocol::{WebUIProtocol, WebUIStream};
use webui_state::find_value_by_dotted_path;

/// Error types for the WebUI handler.
#[derive(Debug, Error)]
pub enum HandlerError {
    #[error("Rendering error: {0}")]
    Rendering(String),

    #[error("Missing stream: {0}")]
    MissingStream(String),

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

/// Context object for processing WebUI streams
struct WebUIProcessContext<'a> {
    protocol: &'a WebUIProtocol,
    state: &'a Value,
    depth: usize,
    writer: &'a mut dyn ResponseWriter,
    // Add local variables map to store context-specific variables (like loop items)
    local_vars: HashMap<String, Value>,
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
        // Start with the main stream (typically "index.html")
        let main_stream_id = "index.html";
        if !protocol.streams.contains_key(main_stream_id) {
            return Err(HandlerError::MissingStream(main_stream_id.to_string()));
        }

        // Process the main stream with an empty initial context
        let mut context = WebUIProcessContext {
            protocol,
            state,
            depth: 0,
            writer,
            local_vars: HashMap::new(),
        };
        self.process_stream_id(main_stream_id, &mut context)?;

        // Finalize the output
        writer.end()?;

        Ok(())
    }

    /// Process a stream by its ID.
    ///
    /// The `context` parameter contains scope-local variables that are accessible during rendering,
    /// such as loop iteration variables. This is separate from the global `state`.
    fn process_stream_id(&self, stream_id: &str, context: &mut WebUIProcessContext) -> Result<()> {
        if let Some(stream) = context.protocol.streams.get(stream_id) {
            self.process_stream(stream, context)
        } else {
            Err(HandlerError::MissingStream(stream_id.to_string()))
        }
    }

    /// Process a vector of streams.
    ///
    /// The `context` maintains scope-specific variables that can be accessed by streams
    /// during rendering, while `state` contains the global application state.
    fn process_stream(
        &self,
        stream: &Vec<WebUIStream>,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        for item in stream {
            match item {
                WebUIStream::Raw(raw) => {
                    context.writer.write(&raw.value)?;
                }
                WebUIStream::Component(component) => {
                    self.process_component(component, context)?;
                }
                WebUIStream::For(for_loop) => {
                    self.process_for_loop(for_loop, context)?;
                }
                WebUIStream::Signal(signal) => {
                    let content = self.process_signal(signal, context)?;
                    context.writer.write(&content)?;
                }
                WebUIStream::If(if_cond) => {
                    self.process_if(if_cond, context)?;
                }
            }
        }
        Ok(())
    }

    /// Process a component stream.
    fn process_component(
        &self,
        component: &webui_protocol::WebUIStreamComponent,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        // Write CSS once per component at the first level
        if context.depth == 0 {
            context.writer.write(&format!(
                "<link rel=\"stylesheet\" href=\"./{}.css\">",
                component.stream_id
            ))?;
        }

        self.process_stream_id(&component.stream_id, context)
    }

    /// Process a for loop stream.
    ///
    /// Creates a new context for each iteration that includes the current loop item.
    /// This allows nested templates to access both the loop variable and any parent context.
    /// Example: `for item in items` makes "item" available in the loop body.
    fn process_for_loop(
        &self,
        for_loop: &webui_protocol::WebUIStreamFor,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        // Get the collection to iterate over
        let collection_name = &for_loop.collection;

        // First check in global state
        let collection =
            if let Some(val) = find_value_by_dotted_path(collection_name, context.state) {
                match val {
                    Value::Array(arr) => arr,
                    _ => {
                        return Err(HandlerError::TypeError(format!(
                            "Collection '{}' is not an array",
                            collection_name
                        )))
                    }
                }
            } else {
                return Err(HandlerError::MissingData(collection_name.to_string()));
            };

        let item_name = &for_loop.item;

        // Process each item in the collection
        for item in collection {
            // Save the current local vars
            let saved_vars = context.local_vars.clone();

            // Add the current item to the context
            context.local_vars.insert(item_name.clone(), item.clone());

            // Process the stream with the updated context
            self.process_stream_id(&for_loop.stream_id, context)?;

            // Restore the original context
            context.local_vars = saved_vars;
        }

        Ok(())
    }

    /// Process a signal stream.
    ///
    /// Looks up the value in the context first (for local variables), then in the global state.
    /// This prioritization allows local variables (like loop items) to override global state.
    fn process_signal(
        &self,
        signal: &webui_protocol::WebUIStreamSignal,
        context: &WebUIProcessContext,
    ) -> Result<String> {
        // Parse the path (could be nested like "person.name")
        let path = &signal.value;

        // First check in local_vars
        if let Some(first_part) = path.split('.').next() {
            if let Some(local_value) = context.local_vars.get(first_part) {
                // If this is a simple path (no dots), just return the value
                if !path.contains('.') {
                    return self.format_signal_value(local_value, signal.raw);
                }

                // Otherwise, use find_value_by_dotted_path starting from the second part
                let remaining_path = &path[first_part.len() + 1..]; // +1 for the dot
                if let Some(nested_value) = find_value_by_dotted_path(remaining_path, local_value) {
                    return self.format_signal_value(&nested_value, signal.raw);
                }
            }
        }

        // If not found in local vars, check in global state
        if let Some(value) = find_value_by_dotted_path(path, context.state) {
            return self.format_signal_value(&value, signal.raw);
        }

        Err(HandlerError::MissingData(path.clone()))
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

    /// Process an if condition stream.
    fn process_if(
        &self,
        if_cond: &webui_protocol::WebUIStreamIf,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        // Evaluate the condition
        let condition_met = evaluate(&if_cond.condition, context.state)
            .map_err(|e| HandlerError::Evaluation(e.to_string()))?;

        if condition_met {
            // Process the content if condition is true
            self.process_stream_id(&if_cond.stream_id, context)?;
        }

        Ok(())
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
        };

        self.process_stream_id("index.html", &mut context)
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
        let mut streams = HashMap::new();
        streams.insert(
            "index.html".to_string(),
            vec![WebUIStream::Raw(webui_protocol::WebUIStreamRaw {
                value: "Hello, WebUI!".to_string(),
            })],
        );

        let protocol = WebUIProtocol { streams };
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
        let mut streams = HashMap::new();
        streams.insert(
            "index.html".to_string(),
            vec![
                WebUIStream::Raw(webui_protocol::WebUIStreamRaw {
                    value: "Hello, ".to_string(),
                }),
                WebUIStream::Signal(webui_protocol::WebUIStreamSignal {
                    value: "name".to_string(),
                    raw: false,
                }),
                WebUIStream::Raw(webui_protocol::WebUIStreamRaw {
                    value: "!".to_string(),
                }),
            ],
        );

        let protocol = WebUIProtocol { streams };
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
        let mut streams = HashMap::new();
        streams.insert(
            "index.html".to_string(),
            vec![
                WebUIStream::Raw(webui_protocol::WebUIStreamRaw {
                    value: "People: ".to_string(),
                }),
                WebUIStream::For(webui_protocol::WebUIStreamFor {
                    item: "person".to_string(),
                    collection: "people".to_string(),
                    stream_id: "person-item".to_string(),
                }),
            ],
        );

        streams.insert(
            "person-item".to_string(),
            vec![
                WebUIStream::Signal(webui_protocol::WebUIStreamSignal {
                    value: "person.name".to_string(),
                    raw: false,
                }),
                WebUIStream::Raw(webui_protocol::WebUIStreamRaw {
                    value: ", ".to_string(),
                }),
            ],
        );

        let protocol = WebUIProtocol { streams };
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
        let mut streams = HashMap::new();
        streams.insert(
            "index.html".to_string(),
            vec![
                WebUIStream::Raw(webui_protocol::WebUIStreamRaw {
                    value: "Status: ".to_string(),
                }),
                WebUIStream::If(webui_protocol::WebUIStreamIf {
                    condition: webui_protocol::ConditionExpr::Identifier {
                        value: "isActive".to_string(),
                    },
                    stream_id: "active-content".to_string(),
                }),
                WebUIStream::Raw(webui_protocol::WebUIStreamRaw {
                    value: "End".to_string(),
                }),
            ],
        );

        streams.insert(
            "active-content".to_string(),
            vec![WebUIStream::Raw(webui_protocol::WebUIStreamRaw {
                value: "Active".to_string(),
            })],
        );

        let protocol = WebUIProtocol { streams };

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
        let mut streams = HashMap::new();
        streams.insert(
            "index.html".to_string(),
            vec![
                WebUIStream::Raw(webui_protocol::WebUIStreamRaw {
                    value: "Component: ".to_string(),
                }),
                WebUIStream::Component(webui_protocol::WebUIStreamComponent {
                    stream_id: "my-component".to_string(),
                }),
            ],
        );

        streams.insert(
            "my-component".to_string(),
            vec![WebUIStream::Raw(webui_protocol::WebUIStreamRaw {
                value: "<div>Component Content</div>".to_string(),
            })],
        );

        let protocol = WebUIProtocol { streams };
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
            "Component: <link rel=\"stylesheet\" href=\"./my-component.css\"><div>Component Content</div>"
        );
        assert!(writer.is_ended());
    }

    #[test]
    fn test_missing_stream() {
        // Create a protocol with a missing stream reference
        let mut streams = HashMap::new();
        streams.insert(
            "index.html".to_string(),
            vec![WebUIStream::Component(
                webui_protocol::WebUIStreamComponent {
                    stream_id: "missing-component".to_string(),
                },
            )],
        );

        let protocol = WebUIProtocol { streams };
        let state = test_json!({});

        // Create a test writer
        let mut writer = TestWriter::new();

        // Handle the protocol
        let result = handle(&protocol, &state, &mut writer);

        // Expect an error
        assert!(result.is_err());
        if let Err(HandlerError::MissingStream(stream_id)) = result {
            assert_eq!(stream_id, "missing-component");
        } else {
            panic!("Expected MissingStream error");
        }
    }
}
