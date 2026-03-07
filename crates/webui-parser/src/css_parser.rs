//! CSS parser for WebUI components.
//!
//! This module uses tree-sitter-css to parse CSS files
//! and process styles for components.

use crate::{ParserError, Result};
use std::collections::HashSet;
use std::fmt;
use tree_sitter::{Node, Parser, Tree};
use webui_protocol::WebUIFragmentRecords;

use tree_sitter_css::LANGUAGE;

/// Parser for CSS files.
pub struct CssParser {
    /// Tree-sitter parser for CSS.
    parser: Parser,
}

impl fmt::Debug for CssParser {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CssParser").finish()
    }
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
        // For now, returning content as-is — future transforms can hook in here.
        // Validation parse is deferred to extract_tokens_and_definitions when
        // called from the HtmlParser, avoiding a redundant tree-sitter parse.
        Ok(style_content.to_string())
    }

    /// Extract CSS custom property token names used via `var()` in the given CSS.
    ///
    /// Returns a set of variable names (without the `--` prefix) that are
    /// **referenced** through `var(--name)` calls. Variables that are only
    /// **defined** (e.g., `--bar: 12px`) in the same CSS are excluded.
    ///
    /// Handles nested fallbacks: `var(--a, var(--b, var(--c)))` yields
    /// `{"a", "b", "c"}` because each nested `var()` is a separate
    /// `call_expression` in the tree-sitter CSS AST.
    pub fn extract_tokens(&mut self, css_content: &str) -> Result<HashSet<String>> {
        let tree = self
            .parser
            .parse(css_content, None)
            .ok_or_else(|| ParserError::Css("Failed to parse CSS for token extraction".into()))?;

        let mut tokens = HashSet::new();
        let mut definitions = HashSet::new();

        Self::walk_css_tree(tree.root_node(), css_content, &mut tokens, &mut definitions);

        // Exclude locally-defined custom properties
        tokens.retain(|t| !definitions.contains(t));
        Ok(tokens)
    }

    /// Extract CSS custom property **definitions** from the given CSS.
    ///
    /// Returns a set of variable names (without `--` prefix) that are
    /// **defined** via `--name: value` declarations. This is used to
    /// exclude application-level token definitions (e.g., from `<style>`
    /// in the entry HTML) from the hoisted token set.
    pub fn extract_definitions(&mut self, css_content: &str) -> Result<HashSet<String>> {
        let tree = self
            .parser
            .parse(css_content, None)
            .ok_or_else(|| ParserError::Css("Failed to parse CSS for definitions".into()))?;

        let mut tokens = HashSet::new();
        let mut definitions = HashSet::new();

        Self::walk_css_tree(tree.root_node(), css_content, &mut tokens, &mut definitions);

        Ok(definitions)
    }

    /// Extract both token **usages** and **definitions** in a single parse.
    ///
    /// Returns `(tokens, definitions)` where:
    /// - `tokens`: var() usages with locally-defined properties already excluded
    /// - `definitions`: all custom property definitions (for global filtering)
    ///
    /// Prefer this over calling `extract_tokens` + `extract_definitions`
    /// separately on the same CSS content to avoid redundant tree-sitter parses.
    pub fn extract_tokens_and_definitions(
        &mut self,
        css_content: &str,
    ) -> Result<(HashSet<String>, HashSet<String>)> {
        let tree = self
            .parser
            .parse(css_content, None)
            .ok_or_else(|| ParserError::Css("Failed to parse CSS".into()))?;

        let mut tokens = HashSet::new();
        let mut definitions = HashSet::new();

        Self::walk_css_tree(tree.root_node(), css_content, &mut tokens, &mut definitions);

        // Exclude locally-defined custom properties from tokens
        tokens.retain(|t| !definitions.contains(t));
        Ok((tokens, definitions))
    }

    /// Iteratively walk the CSS tree to collect var() usages and custom
    /// property definitions. Uses an explicit stack instead of recursion.
    fn walk_css_tree(
        root: Node<'_>,
        source: &str,
        tokens: &mut HashSet<String>,
        definitions: &mut HashSet<String>,
    ) {
        let mut stack = vec![root];

        while let Some(node) = stack.pop() {
            match node.kind() {
                "call_expression" => {
                    Self::extract_var_tokens(node, source, tokens);
                }
                "declaration" => {
                    Self::collect_custom_property_definition(node, source, definitions);
                }
                _ => {}
            }

            // Push children in reverse order for left-to-right traversal
            let count = node.child_count();
            for i in (0..count).rev() {
                if let Some(child) = node.child(i as u32) {
                    stack.push(child);
                }
            }
        }
    }

    /// If `node` is a `var()` call expression, extract its `plain_value`
    /// arguments as token names (stripping the `--` prefix).
    fn extract_var_tokens(node: Node<'_>, source: &str, tokens: &mut HashSet<String>) {
        let count = node.child_count();
        let is_var = (0..count).any(|i| {
            node.child(i as u32).is_some_and(|c| {
                c.kind() == "function_name" && &source[c.start_byte()..c.end_byte()] == "var"
            })
        });

        if !is_var {
            return;
        }

        // Extract plain_value children — the CSS variable references
        let arguments =
            (0..count).find_map(|i| node.child(i as u32).filter(|c| c.kind() == "arguments"));

        if let Some(args) = arguments {
            let arg_count = args.child_count();
            for i in 0..arg_count {
                if let Some(child) = args.child(i as u32) {
                    if child.kind() == "plain_value" {
                        let name = &source[child.start_byte()..child.end_byte()];
                        if let Some(stripped) = name.strip_prefix("--") {
                            tokens.insert(stripped.to_string());
                        }
                    }
                }
            }
        }
    }

    /// If `node` is a declaration with a custom property name (starting
    /// with `--`), record it in the definitions set.
    fn collect_custom_property_definition(
        node: Node<'_>,
        source: &str,
        definitions: &mut HashSet<String>,
    ) {
        if let Some(prop_node) = node.child(0) {
            if prop_node.kind() == "property_name" {
                let prop = &source[prop_node.start_byte()..prop_node.end_byte()];
                if let Some(stripped) = prop.strip_prefix("--") {
                    definitions.insert(stripped.to_string());
                }
            }
        }
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

    // ── extract_tokens tests ────────────────────────────────────────

    #[test]
    fn test_extract_single_var() {
        let mut parser = CssParser::new();
        let tokens = parser
            .extract_tokens(".btn { color: var(--colorPrimary); }")
            .expect("extract_tokens failed");
        assert_eq!(tokens, HashSet::from(["colorPrimary".to_string()]));
    }

    #[test]
    fn test_extract_multiple_vars() {
        let css = r#"
            .btn {
                color: var(--textColor);
                background: var(--bgColor);
                border-radius: var(--radius);
            }
        "#;
        let mut parser = CssParser::new();
        let tokens = parser.extract_tokens(css).expect("extract_tokens failed");
        assert_eq!(
            tokens,
            HashSet::from([
                "textColor".to_string(),
                "bgColor".to_string(),
                "radius".to_string(),
            ])
        );
    }

    #[test]
    fn test_extract_nested_fallback() {
        let css = ".x { color: var(--primary, var(--fallback)); }";
        let mut parser = CssParser::new();
        let tokens = parser.extract_tokens(css).expect("extract_tokens failed");
        assert_eq!(
            tokens,
            HashSet::from(["primary".to_string(), "fallback".to_string()])
        );
    }

    #[test]
    fn test_extract_deeply_nested_fallbacks() {
        let css = ".x { color: var(--a, var(--b, var(--c))); }";
        let mut parser = CssParser::new();
        let tokens = parser.extract_tokens(css).expect("extract_tokens failed");
        assert_eq!(
            tokens,
            HashSet::from(["a".to_string(), "b".to_string(), "c".to_string()])
        );
    }

    #[test]
    fn test_literal_fallback_ignored() {
        let css = ".x { font-size: var(--size, 16px); }";
        let mut parser = CssParser::new();
        let tokens = parser.extract_tokens(css).expect("extract_tokens failed");
        assert_eq!(tokens, HashSet::from(["size".to_string()]));
    }

    #[test]
    fn test_exclude_local_definitions() {
        let css = r#"
            :root { --bar: 12px; }
            .x { width: var(--bar); }
        "#;
        let mut parser = CssParser::new();
        let tokens = parser.extract_tokens(css).expect("extract_tokens failed");
        assert!(
            tokens.is_empty(),
            "Locally defined --bar should be excluded: {tokens:?}"
        );
    }

    #[test]
    fn test_exclude_definitions_keep_unrelated_usages() {
        let css = r#"
            :host { --local: 5px; }
            .x { color: var(--external); }
        "#;
        let mut parser = CssParser::new();
        let tokens = parser.extract_tokens(css).expect("extract_tokens failed");
        assert_eq!(tokens, HashSet::from(["external".to_string()]));
    }

    #[test]
    fn test_empty_css() {
        let mut parser = CssParser::new();
        let tokens = parser.extract_tokens("").expect("extract_tokens failed");
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_no_var_calls() {
        let css = "body { margin: 0; color: red; }";
        let mut parser = CssParser::new();
        let tokens = parser.extract_tokens(css).expect("extract_tokens failed");
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_deduplicate_same_var() {
        let css = r#"
            .a { color: var(--shared); }
            .b { background: var(--shared); }
        "#;
        let mut parser = CssParser::new();
        let tokens = parser.extract_tokens(css).expect("extract_tokens failed");
        assert_eq!(tokens, HashSet::from(["shared".to_string()]));
    }

    #[test]
    fn test_real_world_component_css() {
        let css = r#"
            :host {
                display: inline-flex;
                background-color: var(--colorBrandBackground);
                border-radius: var(--borderRadiusSmall);
                padding: var(--spacingHorizontalM) var(--spacingVerticalS);
                font-family: var(--fontFamilyBase);
                line-height: var(--lineHeightBase400);
            }
            :host(:hover) {
                background-color: var(--colorBrandBackgroundHover);
            }
        "#;
        let mut parser = CssParser::new();
        let tokens = parser.extract_tokens(css).expect("extract_tokens failed");
        assert_eq!(
            tokens,
            HashSet::from([
                "colorBrandBackground".to_string(),
                "borderRadiusSmall".to_string(),
                "spacingHorizontalM".to_string(),
                "spacingVerticalS".to_string(),
                "fontFamilyBase".to_string(),
                "lineHeightBase400".to_string(),
                "colorBrandBackgroundHover".to_string(),
            ])
        );
    }

    #[test]
    fn test_extract_tokens_and_definitions() {
        let css = r#"
            :root { --brand: #0078d4; --radius: 6px; }
            .btn { color: var(--brand); margin: var(--external); }
        "#;
        let mut parser = CssParser::new();
        let (tokens, defs) = parser.extract_tokens_and_definitions(css).expect("failed");

        // tokens should exclude locally-defined --brand but keep --external
        assert_eq!(tokens, HashSet::from(["external".to_string()]));
        // definitions should include both defined properties
        assert_eq!(
            defs,
            HashSet::from(["brand".to_string(), "radius".to_string()])
        );
    }

    #[test]
    fn test_extract_definitions_only() {
        let css = ":root { --color-primary: #0078d4; --spacing-m: 12px; }";
        let mut parser = CssParser::new();
        let defs = parser.extract_definitions(css).expect("failed");
        assert_eq!(
            defs,
            HashSet::from(["color-primary".to_string(), "spacing-m".to_string()])
        );
    }

    #[test]
    fn test_extract_definitions_none_when_no_custom_props() {
        let css = "body { margin: 0; color: red; }";
        let mut parser = CssParser::new();
        let defs = parser.extract_definitions(css).expect("failed");
        assert!(defs.is_empty());
    }

    #[test]
    fn test_extract_tokens_malformed_var_missing_dashes() {
        // var(-primary) is not a valid custom property reference (needs --)
        let css = ".x { color: var(-primary); }";
        let mut parser = CssParser::new();
        let tokens = parser.extract_tokens(css).expect("failed");
        // Should not extract since it doesn't start with --
        assert!(
            tokens.is_empty(),
            "Single-dash var should not be extracted: {tokens:?}"
        );
    }

    #[test]
    fn test_extract_tokens_empty_var() {
        let css = ".x { color: var(); }";
        let mut parser = CssParser::new();
        let tokens = parser.extract_tokens(css).expect("failed");
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_extract_tokens_definitions_only_css() {
        // CSS that only defines custom properties, no var() usage
        let css = ":root { --a: 1px; --b: 2px; --c: red; }";
        let mut parser = CssParser::new();
        let tokens = parser.extract_tokens(css).expect("failed");
        assert!(
            tokens.is_empty(),
            "Definitions-only CSS should yield no tokens"
        );
    }
}
