# WebUI Rust Handler

The WebUI Rust handler provides high-performance rendering of WebUI protocols in Rust applications.

## Installation

Add the WebUI crates to your `Cargo.toml`:

```toml
[dependencies]
webui-protocol = "0.1.0"
webui-handler = "0.1.0"
webui-expressions = "0.1.0"
webui-state = "0.1.0"
```

## Basic Usage

```rust
use webui_protocol::WebUIProtocol;
use serde_json::{json, Value};
use webui_handler::{handle, ResponseWriter, Result};
use webui_protocol::WebUIProtocol;

// Define a simple response writer
struct StringWriter {
    content: String,
}

impl StringWriter {
    fn new() -> Self {
        Self { content: String::new() }
    }
}

impl ResponseWriter for StringWriter {
    fn write(&mut self, content: &str) -> Result<()> {
        self.content.push_str(content);
        Ok(())
    }
    
    fn end(&mut self) -> Result<()> {
        // Nothing to do for string writer
        Ok(())
    }
}

fn main() -> Result<()> {
    // Load protocol from file
    let protocol = WebUIProtocol::from_protobuf_file("template.bin")?;
    
    // Create state data
    let state = json!({
        "title": "WebUI Example",
        "items": [
            {"name": "Item 1", "description": "First item"},
            {"name": "Item 2", "description": "Second item"},
            {"name": "Item 3", "description": "Third item"}
        ]
    });
    
    // Create writer
    let mut writer = StringWriter::new();
    
    // Render the protocol
    handle(&protocol, &state, &mut writer)?;
    
    // Use the rendered HTML
    println!("Rendered HTML: {}", writer.content);
    
    Ok(())
}
```

## Stream-Based Writing

For high-performance use cases, you can implement the `ResponseWriter` trait for streaming output:

```rust
use std::io::{self, Write};
use webui_handler::{ResponseWriter, Result};

struct StreamWriter<W: Write> {
    writer: W,
}

impl<W: Write> StreamWriter<W> {
    fn new(writer: W) -> Self {
        Self { writer }
    }
}

impl<W: Write> ResponseWriter for StreamWriter<W> {
    fn write(&mut self, content: &str) -> Result<()> {
        self.writer.write_all(content.as_bytes())?;
        Ok(())
    }
    
    fn end(&mut self) -> Result<()> {
        self.writer.flush()?;
        Ok(())
    }
}

// Usage with any Write implementation:
let file = File::create("output.html")?;
let mut writer = StreamWriter::new(file);
handle(&protocol, &state, &mut writer)?;
```

## WebUI Handler API

The main entry point is the `WebUIHandler` struct:

```rust
pub struct WebUIHandler {
    plugin: Option<Box<dyn HandlerPlugin>>,
}

impl WebUIHandler {
    /// Create a handler with no plugin (plain HTML rendering).
    pub fn new() -> Self;

    /// Create a handler with a plugin for framework-specific rendering.
    pub fn with_plugin(plugin: Box<dyn HandlerPlugin>) -> Self;

    /// Render the protocol with state data.
    pub fn handle(
        &mut self,
        protocol: &WebUIProtocol,
        state: &Value,
        writer: &mut dyn ResponseWriter,
    ) -> Result<()>;
}
```

### Parameters

- `protocol`: The WebUI protocol to render
- `state`: A JSON value containing the data for rendering
- `writer`: An implementation of the `ResponseWriter` trait for output

### ResponseWriter Trait

```rust
pub trait ResponseWriter {
    fn write(&mut self, content: &str) -> Result<()>;
    fn end(&mut self) -> Result<()>;
}
```

## Using Plugins

The handler supports optional plugins that inject framework-specific content during rendering. Use `with_plugin` to create a handler with a plugin:

```rust
use webui_handler::{WebUIHandler, plugin::fast::FastHydrationPlugin};

let mut handler = WebUIHandler::with_plugin(Box::new(FastHydrationPlugin::new()));
handler.handle(&protocol, &state, &mut writer)?;
```

When a plugin is loaded, the handler calls plugin hooks at key rendering points:
- Before/after signals, loops, and conditionals
- Before/after each loop iteration
- When entering/leaving component scopes
- When processing `Plugin` protocol fragments

See [Plugins](/guide/concepts/plugins/) for the full plugin API and how to write custom plugins.

## Error Handling

The WebUI handler provides detailed error types through the `HandlerError` enum:

```rust
pub enum HandlerError {
    Rendering(String),
    MissingFragment(String),
    MissingData(String),
    TypeError(String),
    Protocol(webui_protocol::ProtocolError),
    Evaluation(String),
    Io(std::io::Error),
    Writer(String),
}
```

You can handle these specific error cases to provide better error messages for different failure scenarios.
