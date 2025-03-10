//! WebUI Parser - A parser for WebUI components and templates
//! 
//! This library scans directories for HTML and CSS files, parses them, and
//! converts them into the WebUI protocol format.

mod component_registry;
mod html_parser;
mod css_parser;
mod condition_parser;
mod error;
mod handlebars_parser;

pub use component_registry::{Component, ComponentRegistry};
pub use condition_parser::ConditionParser;
pub use html_parser::HtmlParser;
pub use css_parser::CssParser;
pub use error::{ParserError, Result};
pub use handlebars_parser::HandlebarsParser;

use std::path::Path;
use webui_protocol::{WebUIProtocol, WebUIStreamRecords};
use std::fs;

/// Main parser for WebUI components and templates.
pub struct WebUIParser {
    /// Registry for web components.
    component_registry: ComponentRegistry,
    
    /// Directive parser for WebUI directives.
    html_parser: HtmlParser,
}

impl WebUIParser {
    /// Create a new WebUI parser.
    pub fn new() -> Self {
        let stream_records = WebUIStreamRecords::new();
        Self {
            component_registry: ComponentRegistry::new(),
            html_parser: HtmlParser::new(stream_records),
        }
    }
    
    /// Parse the entry point file to create the protocol.
    pub fn parse<P: AsRef<Path>>(mut self, entry_file: P, directories: &[P]) -> Result<WebUIProtocol> {
        // Register components from the provided directories
        self.component_registry.register_from_paths(directories)?;
        
        let entry_path = entry_file.as_ref();
        let tag_name = entry_path
            .file_stem()
            .ok_or_else(|| ParserError::NotFound("Invalid entry file name".to_string()))?
            .to_str()
            .ok_or_else(|| ParserError::NotFound("Invalid UTF8 in entry file name".to_string()))?;
        
        // Check if the entry file is registered as a component
        if !self.component_registry.contains(tag_name) {
            return Err(ParserError::NotFound(format!(
                "Entry file '{}' not found in component registry", tag_name
            )));
        }
        
        // Read entry file content
        let html_content = fs::read_to_string(entry_path)
            .map_err(|e| ParserError::IO(format!("Failed to read entry file: {}", e)))?;
        
        // Parse HTML and generate streams
        match self.html_parser.parse(&format!("{}.html", tag_name), &html_content) {
            Ok(_) => Ok(webui_protocol::WebUIProtocol { streams: self.html_parser.into_stream_records() }),
            Err(error) => Err(error),
        }

    }
}

impl Default for WebUIParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_simple_parse() {
        let parser = WebUIParser::new();
        let result = parser.parse("Hello World", &["./test_dir"]);
        assert!(result.is_ok());
    }
    
    #[tokio::test]
    async fn test_parse_example() {
        let html = r#"
            Hello, WebUI!
            <for condition="person in people">
                <p>{{person.name}}</p>
            </for>
            {{{raw_description}}}
            <if condition="contact">
                Hello, {{name}}
            </if>
        "#;
        
        let parser = WebUIParser::new();
        let result= parser.parse(html, &["./test_dir"]);
        assert!(result.is_ok());
        assert!(result.unwrap().streams.contains_key("index.html"));
    }
}
