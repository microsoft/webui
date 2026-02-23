use std::error::Error;
use std::fs;

use serde_json::Value;
use webui_handler::{ResponseWriter, WebUIHandler};
use webui_parser::HtmlParser;
use webui_protocol::WebUIProtocol;

use crate::config::AppPaths;

struct MemoryWriter {
    content: String,
}

impl MemoryWriter {
    fn new() -> Self {
        Self {
            content: String::new(),
        }
    }
}

impl ResponseWriter for MemoryWriter {
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        self.content.push_str(content);
        Ok(())
    }

    fn end(&mut self) -> webui_handler::Result<()> {
        Ok(())
    }
}

pub fn render_to_index_html(paths: &AppPaths) -> Result<(), Box<dyn Error + Send + Sync>> {
    // Load and parse the template into a WebUI protocol
    let template = fs::read_to_string(&paths.template)?;
    let mut parser = HtmlParser::new();
    parser.parse("index.html", &template)?;
    let fragments = parser.into_fragment_records();
    let protocol = WebUIProtocol { fragments };

    // Load the state from state.json
    let state_json = fs::read_to_string(&paths.data)?;
    let state: Value = serde_json::from_str(&state_json)?;

    // Render into an in-memory buffer
    let mut writer = MemoryWriter::new();
    let handler = WebUIHandler::new();
    handler.handle(&protocol, &state, &mut writer)?;

    // Ensure the dist directory exists and write to dist/index.html
    fs::create_dir_all(&paths.dist_dir)?;
    fs::write(paths.dist_dir.join("index.html"), &writer.content)?;

    Ok(())
}
