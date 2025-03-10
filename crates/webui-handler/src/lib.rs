//! WebUI Handler implementation for Rust.
//!
//! This crate provides functionality to process and render WebUI protocols
//! into final HTML output based on provided data.

use std::collections::HashMap;
use thiserror::Error;
use serde_json::Value;
use webui_protocol::{WebUIProtocol, WebUIStream};
use webui_expressions::evaluate_condition;

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
}

pub type Result<T> = std::result::Result<T, HandlerError>;

/// The main WebUI handler that processes protocols and renders them.
pub struct WebUIHandler {
    // Configuration options could go here
}

impl WebUIHandler {
    /// Create a new WebUI handler.
    pub fn new() -> Self {
        Self {}
    }
    
    /// Render a protocol with the provided data.
    pub fn render(&self, protocol: &WebUIProtocol, data: &HashMap<String, Value>) -> Result<String> {
        // Start with the main stream (typically "index.html")
        let main_stream_id = "index.html";
        if !protocol.streams.contains_key(main_stream_id) {
            return Err(HandlerError::MissingStream(main_stream_id.to_string()));
        }
        
        // Process the main stream
        self.process_stream_id(main_stream_id, protocol, data, &mut HashMap::new())
    }
    
    /// Process a stream by its ID.
    fn process_stream_id(
        &self,
        stream_id: &str,
        protocol: &WebUIProtocol,
        data: &HashMap<String, Value>,
        context: &mut HashMap<String, Value>,
    ) -> Result<String> {
        let stream = protocol.streams.get(stream_id).ok_or_else(|| {
            HandlerError::MissingStream(stream_id.to_string())
        })?;
        
        self.process_stream(stream, protocol, data, context)
    }
    
    /// Process a vector of streams.
    fn process_stream(
        &self,
        stream: &Vec<WebUIStream>,
        protocol: &WebUIProtocol,
        data: &HashMap<String, Value>,
        context: &mut HashMap<String, Value>,
    ) -> Result<String> {
        let mut result = String::new();
        
        for item in stream {
            let content = match item {
                WebUIStream::Raw(raw) => raw.value.clone(),
                WebUIStream::Component(component) => {
                    self.process_component(component, protocol, data, context)?
                },
                WebUIStream::For(for_loop) => {
                    self.process_for_loop(for_loop, protocol, data, context)?
                },
                WebUIStream::Signal(signal) => {
                    self.process_signal(signal, data, context)?
                },
                WebUIStream::If(if_cond) => {
                    self.process_if(if_cond, protocol, data, context)?
                },
            };
            
            result.push_str(&content);
        }
        
        Ok(result)
    }
    
    /// Process a component stream.
    fn process_component(
        &self,
        component: &webui_protocol::WebUIStreamComponent,
        protocol: &WebUIProtocol,
        data: &HashMap<String, Value>,
        context: &mut HashMap<String, Value>,
    ) -> Result<String> {
        // In a real implementation, we would process the CSS here
        // For now, we just process the referenced stream
        self.process_stream_id(&component.stream_id, protocol, data, context)
    }
    
    /// Process a for loop stream.
    fn process_for_loop(
        &self,
        for_loop: &webui_protocol::WebUIStreamFor,
        protocol: &WebUIProtocol,
        data: &HashMap<String, Value>,
        parent_context: &mut HashMap<String, Value>,
    ) -> Result<String> {
        // Get the collection to iterate over
        let collection_name = &for_loop.collection;
        let collection = match parent_context.get(collection_name).or_else(|| data.get(collection_name)) {
            Some(Value::Array(arr)) => arr,
            Some(_) => return Err(HandlerError::TypeError(format!(
                "Collection '{}' is not an array", collection_name
            ))),
            None => return Err(HandlerError::MissingData(collection_name.to_string())),
        };
        
        let mut result = String::new();
        let item_name = &for_loop.item;
        
        // Process each item in the collection
        for item in collection {
            let mut item_context = parent_context.clone();
            item_context.insert(item_name.to_string(), item.clone());
            
            let content = self.process_stream_id(&for_loop.stream_id, protocol, data, &mut item_context)?;
            result.push_str(&content);
        }
        
        Ok(result)
    }
    
    /// Process a signal stream.
    fn process_signal(
        &self,
        signal: &webui_protocol::WebUIStreamSignal,
        data: &HashMap<String, Value>,
        context: &mut HashMap<String, Value>,
    ) -> Result<String> {
        // Parse the path (could be nested like "person.name")
        let path_parts: Vec<&str> = signal.value.split('.').collect();
        let mut current_value = None;
        
        // First check in context, then in data
        if let Some(value) = context.get(path_parts[0]).or_else(|| data.get(path_parts[0])) {
            current_value = Some(value);
            
            // Navigate through nested properties
            for &part in path_parts.iter().skip(1) {
                match current_value {
                    Some(Value::Object(obj)) => {
                        current_value = obj.get(part);
                    },
                    _ => return Err(HandlerError::TypeError(format!(
                        "Cannot access property '{}' on non-object", part
                    ))),
                }
            }
        }
        
        match current_value {
            Some(value) => {
                let result = if signal.raw {
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
            },
            None => Err(HandlerError::MissingData(signal.value.clone())),
        }
    }
    
    /// Process an if condition stream.
    fn process_if(
        &self,
        if_cond: &webui_protocol::WebUIStreamIf,
        protocol: &WebUIProtocol,
        data: &HashMap<String, Value>,
        context: &mut HashMap<String, Value>,
    ) -> Result<String> {
        // Evaluate the condition
        let condition_met = evaluate_condition(&if_cond.condition, data, context)
            .map_err(|e| HandlerError::Evaluation(e.to_string()))?;
        
        if condition_met {
            // Process the content if condition is true
            self.process_stream_id(&if_cond.stream_id, protocol, data, context)
        } else {
            // Return empty string if condition is false
            Ok(String::new())
        }
    }
}

impl Default for WebUIHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_render_raw() {
        let handler = WebUIHandler::new();
        
        let mut streams = HashMap::new();
        streams.insert("index.html".to_string(), vec![
            WebUIStream::Raw(webui_protocol::WebUIStreamRaw {
                value: "Hello, WebUI!".to_string(),
            }),
        ]);
        
        let protocol = WebUIProtocol { streams };
        let data = HashMap::new();
        
        let result = handler.render(&protocol, &data).unwrap();
        assert_eq!(result, "Hello, WebUI!");
    }
    
    #[test]
    fn test_render_signal() {
        let handler = WebUIHandler::new();
        
        let mut streams = HashMap::new();
        streams.insert("index.html".to_string(), vec![
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
        ]);
        
        let protocol = WebUIProtocol { streams };
        let mut data = HashMap::new();
        data.insert("name".to_string(), json!("WebUI"));
        
        let result = handler.render(&protocol, &data).unwrap();
        assert_eq!(result, "Hello, WebUI!");
    }
    
    // More tests would be added for for loops, components, and if conditions
}
