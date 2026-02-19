//! CSS parser for WebUI components.
//!
//! This module uses tree-sitter-css to parse CSS files
//! and process styles for components.

use crate::{ParserError, Result};
use tree_sitter::{Parser, Tree};
use webui_protocol::WebUIFragmentRecords;

// Replace extern "C" block with proper import
use tree_sitter_css::LANGUAGE;

/// Parser for CSS files.
pub struct CssParser {
    /// Tree-sitter parser for CSS.
    parser: Parser,
}

impl CssParser {
    /// Create a new CSS parser.
    pub fn new() -> Self {
        let mut parser = Parser::new();
        parser
            .set_language(&LANGUAGE.into())
            .expect("Error loading CSS grammar");

        Self { parser }
    }

    /// Parse CSS content and return fragments.
    pub fn parse(&mut self, css_content: &str) -> Result<WebUIFragmentRecords> {
        // Parse CSS with tree-sitter
        let _tree = self
            .parser
            .parse(css_content, None)
            .ok_or_else(|| ParserError::Css("Failed to parse CSS".to_string()))?;

        // Create empty fragments for now
        // In a real implementation, would convert CSS to fragments
        let fragments = WebUIFragmentRecords::new();

        Ok(fragments)
    }

    /// Process CSS content and merge it into fragments.
    pub fn process_css(
        &mut self,
        css_content: &str,
        fragments: &mut WebUIFragmentRecords,
    ) -> Result<()> {
        // Parse CSS with tree-sitter
        let tree = self
            .parser
            .parse(css_content, None)
            .ok_or_else(|| ParserError::Css("Failed to parse CSS".to_string()))?;

        // Extract and process CSS rules
        self.process_css_rules(tree, css_content, fragments)?;

        Ok(())
    }

    /// Process CSS rules from the parsed tree.
    fn process_css_rules(
        &self,
        _tree: Tree,
        _source: &str,
        _fragments: &mut WebUIFragmentRecords,
    ) -> Result<()> {
        // This is a placeholder for CSS processing logic
        // In a real implementation, you would:
        // 1. Extract selectors and rules from the CSS
        // 2. Associate styles with components
        // 3. Update component fragments with the appropriate styles

        // For now, we'll just return Ok as a placeholder
        Ok(())
    }

    /// Parse inline CSS from style tags in HTML.
    pub fn parse_inline_css(&mut self, style_content: &str) -> Result<String> {
        // Parse the CSS content
        let _tree = self
            .parser
            .parse(style_content, None)
            .ok_or_else(|| ParserError::Css("Failed to parse inline CSS".to_string()))?;

        // For simplicity, we're just returning the content as-is
        // In a real implementation, you might want to process and transform it
        Ok(style_content.to_string())
    }
}

impl Default for CssParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_parse_simple_css() {
        let css = r#"
            body {
                margin: 0;
                padding: 0;
            }
            
            .container {
                width: 100%;
                max-width: 1200px;
                margin: 0 auto;
            }
        "#;

        let mut parser = CssParser::new();
        let result = parser.process_css(css, &mut HashMap::new());
        assert!(result.is_ok());
    }
}
