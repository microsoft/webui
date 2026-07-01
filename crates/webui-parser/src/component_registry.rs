// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Component registry for WebUI framework.
//!
//! This module manages the registry of web components used in the application.

use crate::{CssFallbackChain, CssParser, LegalComments, ParserError, Result};
use std::collections::HashMap;
#[cfg(feature = "fs")]
use std::fs;
#[cfg(feature = "fs")]
use std::path::Path;
use std::path::PathBuf;
#[cfg(feature = "fs")]
use walkdir::WalkDir;

type ProcessedCss = (String, Vec<String>, Vec<String>, Vec<CssFallbackChain>);
#[cfg(feature = "fs")]
const COMPONENT_SCRIPT_EXTENSIONS: &[&str] = &["ts", "tsx", "js", "jsx", "mjs"];

/// Represents a web component in the registry.
#[derive(Debug, Clone)]
pub struct Component {
    /// The custom element tag name (e.g., "hello-world")
    pub tag_name: String,

    /// The HTML content of the component
    pub html_content: String,

    /// The CSS content of the component, if any
    pub css_content: Option<String>,

    /// CSS custom property token names extracted from this component's CSS
    /// (sorted, deduplicated, without `--` prefix).
    pub css_tokens: Vec<String>,

    /// CSS custom property definitions from this component's CSS.
    pub css_definitions: Vec<String>,

    /// CSS `var()` fallback chains from this component's CSS.
    pub css_fallback_chains: Vec<CssFallbackChain>,

    /// The file path where this component is defined
    pub source_path: PathBuf,

    /// The class name that implements this component (if available)
    pub class_name: Option<String>,

    /// Whether this component has an authored client script next to its HTML.
    pub has_script: bool,
}

/// Registry of web components.
#[derive(Debug)]
pub struct ComponentRegistry {
    /// Map of component tag names to their component data
    components: HashMap<String, Component>,
    /// Reusable CSS parser for token extraction during registration.
    css_parser: CssParser,
    /// Legal comment preservation policy for component CSS.
    legal_comments: LegalComments,
}

#[cfg(feature = "fs")]
fn component_has_script(html_path: &Path) -> bool {
    for extension in COMPONENT_SCRIPT_EXTENSIONS {
        if html_path.with_extension(extension).exists() {
            return true;
        }
    }
    false
}

impl Default for ComponentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ComponentRegistry {
    /// Create a new component registry.
    pub fn new() -> Self {
        Self::with_legal_comments(LegalComments::default())
    }

    pub(crate) fn with_legal_comments(legal_comments: LegalComments) -> Self {
        Self {
            components: HashMap::new(),
            css_parser: CssParser::new(),
            legal_comments,
        }
    }

    /// Register multiple components from directories recursively.
    #[cfg(feature = "fs")]
    pub fn register_from_paths<P: AsRef<Path>>(&mut self, directories: &[P]) -> Result<&mut Self> {
        for dir in directories {
            for entry in WalkDir::new(dir.as_ref())
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                // Only process HTML files
                if path.extension().is_some_and(|ext| ext == "html") {
                    // Check for a component name (must contain a hyphen)
                    if let Some(filename) = path.file_stem().and_then(|s| s.to_str()) {
                        if filename.contains('-') {
                            // Find associated CSS file
                            let css_path = path.with_extension("css");
                            // Register the component (key is the file name without extension)
                            self.register_component_from_paths(
                                path,
                                if css_path.exists() {
                                    Some(&css_path)
                                } else {
                                    None
                                },
                            )?;
                        }
                    }
                }
            }
        }
        Ok(self)
    }

    /// Register a web component from paths to HTML and CSS files.
    #[cfg(feature = "fs")]
    pub fn register_component_from_paths<P: AsRef<Path>, Q: AsRef<Path>>(
        &mut self,
        html_path: P,
        css_path: Option<Q>,
    ) -> Result<()> {
        let html_path = html_path.as_ref();

        // Extract component name from file name (without extension)
        let tag_name = html_path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| ParserError::Component("Invalid component file name".to_string()))?;

        // Validate component name (must contain a hyphen)
        if !tag_name.contains('-') {
            return Err(ParserError::Component(format!(
                "Component name '{}' must contain a hyphen",
                tag_name
            )));
        }

        // Check for duplicate component
        if self.components.contains_key(tag_name) {
            return Err(ParserError::Component(format!(
                "Component '{}' is already registered",
                tag_name
            )));
        }

        // Read HTML content
        let html_content = fs::read_to_string(html_path).map_err(|source| ParserError::IO {
            context: format!("Failed to read HTML file: {}", html_path.display()),
            source,
        })?;

        // Read CSS content and extract tokens if available
        let (css_content, css_tokens, css_definitions, css_fallback_chains) =
            if let Some(css_path) = css_path {
                let css_path = css_path.as_ref();
                if css_path.exists() {
                    let content =
                        fs::read_to_string(css_path).map_err(|source| ParserError::IO {
                            context: format!("Failed to read CSS file: {}", css_path.display()),
                            source,
                        })?;
                    let (content, tokens, definitions, requirements) =
                        self.process_css_content(&content)?;
                    (Some(content), tokens, definitions, requirements)
                } else {
                    (None, Vec::new(), Vec::new(), Vec::new())
                }
            } else {
                (None, Vec::new(), Vec::new(), Vec::new())
            };

        // Create and register the component
        let component = Component {
            tag_name: tag_name.to_string(),
            html_content,
            css_content,
            css_tokens,
            css_definitions,
            css_fallback_chains,
            source_path: html_path.to_path_buf(),
            class_name: None,
            has_script: component_has_script(html_path),
        };

        self.components.insert(tag_name.to_string(), component);
        Ok(())
    }

    /// Register a component directly from provided content strings.
    pub fn register_component(
        &mut self,
        tag_name: &str,
        html_content: &str,
        css_content: Option<&str>,
    ) -> Result<()> {
        // Validate component name (must contain a hyphen)
        if !tag_name.contains('-') {
            return Err(ParserError::Component(format!(
                "Component name '{}' must contain a hyphen",
                tag_name
            )));
        }

        // Check for duplicate component
        if self.components.contains_key(tag_name) {
            return Err(ParserError::Component(format!(
                "Component '{}' is already registered",
                tag_name
            )));
        }

        // Extract CSS tokens if CSS content is provided
        let (css_content, css_tokens, css_definitions, css_fallback_chains) = match css_content {
            Some(css) => {
                let (content, tokens, definitions, requirements) = self.process_css_content(css)?;
                (Some(content), tokens, definitions, requirements)
            }
            None => (None, Vec::new(), Vec::new(), Vec::new()),
        };

        // Create component with dummy path since it's coming from string content
        let component: Component = Component {
            tag_name: tag_name.to_string(),
            html_content: html_content.to_string(),
            css_content,
            css_tokens,
            css_definitions,
            css_fallback_chains,
            source_path: PathBuf::new(), // Empty path since it's not from a file
            class_name: None,
            has_script: false,
        };

        // Register the component
        self.components.insert(tag_name.to_string(), component);
        Ok(())
    }

    /// Extract CSS tokens from content and return as a sorted `Vec`.
    fn process_css_content(&mut self, css_content: &str) -> Result<ProcessedCss> {
        let (tokens, definitions, requirements, stripped) = self
            .css_parser
            .extract_tokens_definitions_requirements_and_strip_comments(
                css_content,
                self.legal_comments,
            )?;
        let mut sorted: Vec<String> = tokens.into_iter().collect();
        sorted.sort();
        let mut sorted_definitions: Vec<String> = definitions.into_iter().collect();
        sorted_definitions.sort();
        Ok((
            stripped.into_owned(),
            sorted,
            sorted_definitions,
            requirements,
        ))
    }

    /// Check if a tag name is registered as a component.
    pub fn contains(&self, tag_name: &str) -> bool {
        self.components.contains_key(tag_name)
    }

    /// Get a component by its tag name.
    pub fn get(&self, tag_name: &str) -> Option<&Component> {
        self.components.get(tag_name)
    }

    /// Get all registered components.
    pub fn get_all(&self) -> impl Iterator<Item = &Component> {
        self.components.values()
    }

    /// Iterate the registered component tag names (e.g. `mp-button`).
    ///
    /// Used to offer "did you mean …?" suggestions when an unknown component
    /// tag is encountered during parsing.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.components.keys().map(String::as_str)
    }

    /// Get the number of registered components.
    pub fn len(&self) -> usize {
        self.components.len()
    }

    /// Check if the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.components.is_empty()
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use webui_test_utils::TestFileSystem;

    #[test]
    fn test_register_component() {
        let html_content = "<p>Hello World</p>";
        let css_content = "p { color: red; }";

        // Create temporary files with proper names directly
        let mut fs = TestFileSystem::new();
        let html_path = fs.add_file("components/test-component.html", html_content);
        let css_path = fs.add_file("components/test-component.css", css_content);

        // Register the component (no rename needed)
        let mut registry = ComponentRegistry::new();
        let result = registry.register_component_from_paths(&html_path, Some(&css_path));

        assert!(result.is_ok());
        assert!(registry.contains("test-component"));

        let component = registry
            .get("test-component")
            .expect("Failed to retrieve registered component");
        assert_eq!(component.html_content, html_content);
        assert_eq!(component.css_content.as_deref(), Some(css_content));
        assert!(!component.has_script);
    }

    #[test]
    fn test_register_component_detects_sibling_script() {
        let mut fs = TestFileSystem::new();
        let html_path = fs.add_file("components/scripted-card.html", "<p>Scripted</p>");
        std::fs::write(html_path.with_extension("ts"), "export {};")
            .expect("Failed to write sibling script");

        let mut registry = ComponentRegistry::new();
        registry
            .register_component_from_paths(&html_path, None::<&str>)
            .expect("register failed");

        let component = registry
            .get("scripted-card")
            .expect("Failed to retrieve registered component");
        assert!(component.has_script);
    }

    #[test]
    fn test_component_name_validation() {
        let html_content = "<p>Invalid</p>";

        // Create temporary file with invalid name (no hyphen)
        let mut fs = TestFileSystem::new();
        let html_path = fs.add_file("invalid_name.html", html_content);

        // Try to register the component
        let mut registry = ComponentRegistry::new();
        let result = registry.register_component_from_paths(&html_path, None::<&str>);

        assert!(result.is_err());
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn test_missing_css_file() {
        let html_content = "<p>CSS Optional</p>";

        // Create temporary HTML file
        let mut fs = TestFileSystem::new();
        let html_path = fs.add_file("test-component.html", html_content);

        // Register with non-existent CSS file
        let mut registry = ComponentRegistry::new();
        let result = registry.register_component_from_paths(&html_path, None::<&str>);

        assert!(result.is_ok());
        let component = registry
            .get("test-component")
            .expect("Failed to retrieve registered component");
        assert_eq!(component.html_content, html_content);
        assert_eq!(component.css_content, None);
    }

    #[test]
    fn test_register_component_from_strings() {
        let html_content = "<p>Hello from string!</p>";
        let css_content = "p { color: green; }";

        // Register component directly from strings
        let mut registry = ComponentRegistry::new();
        let result =
            registry.register_component("string-component", html_content, Some(css_content));

        assert!(result.is_ok());
        assert!(registry.contains("string-component"));

        let component = registry
            .get("string-component")
            .expect("Failed to retrieve registered component");
        assert_eq!(component.html_content, html_content);
        assert_eq!(component.css_content.as_deref(), Some(css_content));
    }

    #[test]
    fn test_register_component_strips_css_comments() {
        let mut registry = ComponentRegistry::new();
        registry
            .register_component(
                "style-component",
                "<p>Styled</p>",
                Some("/* var(--ignored) */ p { color: var(--textColor); } /* remove */"),
            )
            .expect("register failed");

        let component = registry
            .get("style-component")
            .expect("Failed to retrieve registered component");
        assert_eq!(
            component.css_content.as_deref(),
            Some(" p { color: var(--textColor); } ")
        );
        assert_eq!(component.css_tokens, vec!["textColor"]);
    }

    #[test]
    fn test_register_component_preserves_legal_css_comments_by_default() {
        let mut registry = ComponentRegistry::new();
        registry
            .register_component(
                "legal-component",
                "<p>Styled</p>",
                Some("/*! @license MIT */ p { color: red; } /* remove */"),
            )
            .expect("register failed");

        let component = registry
            .get("legal-component")
            .expect("Failed to retrieve registered component");
        assert_eq!(
            component.css_content.as_deref(),
            Some("/*! @license MIT */ p { color: red; } ")
        );
    }

    #[test]
    fn test_register_component_strips_legal_css_comments_when_disabled() {
        let mut registry = ComponentRegistry::with_legal_comments(LegalComments::None);
        registry
            .register_component(
                "legal-component",
                "<p>Styled</p>",
                Some("/*! @license MIT */ p { color: red; }"),
            )
            .expect("register failed");

        let component = registry
            .get("legal-component")
            .expect("Failed to retrieve registered component");
        assert_eq!(component.css_content.as_deref(), Some(" p { color: red; }"));
    }

    #[test]
    fn test_invalid_component_name_from_strings() {
        let html_content = "<p>Invalid component</p>";

        // Try registering with invalid name (no hyphen)
        let mut registry = ComponentRegistry::new();
        let result = registry.register_component("invalid", html_content, None);

        // More idiomatic approach using assert!() with message
        assert!(result.is_err(), "Expected error for invalid component name");

        // Better pattern matching using matches!() macro
        assert!(
            matches!(result, Err(ParserError::Component(ref msg)) if msg.contains("must contain a hyphen")),
            "Wrong error type or message: {:?}",
            result
        );
    }

    #[test]
    fn test_duplicate_component_from_strings() {
        let html_content = "<p>First component</p>";
        let html_content2 = "<p>Second component with same name</p>";

        // Register the first component
        let mut registry = ComponentRegistry::new();
        let result1 = registry.register_component("dupe-component", html_content, None);
        assert!(result1.is_ok());

        // Try to register a second component with the same name
        let result2 = registry.register_component("dupe-component", html_content2, None);
        assert!(result2.is_err());

        // Verify the error message
        assert!(
            matches!(result2, Err(ParserError::Component(ref msg)) if msg.contains("already registered")),
            "Expected 'already registered' error, got: {:?}",
            result2
        );

        // Verify the original component is still there unchanged
        let component = registry
            .get("dupe-component")
            .expect("Failed to retrieve registered component");
        assert_eq!(component.html_content, html_content);
    }

    #[test]
    fn test_duplicate_component_from_paths() {
        let html_content1 = "<p>Component from dir A</p>";
        let html_content2 = "<p>Component from dir B</p>";

        // Create temporary directories and files.
        let mut fs = TestFileSystem::new();
        let file_path_a = fs.add_file("dir_a/my-comp.html", html_content1);
        let file_path_b = fs.add_file("dir_b/my-comp.html", html_content2);

        // Register the first component
        let mut registry = ComponentRegistry::new();
        let result1 = registry.register_component_from_paths(&file_path_a, None::<&str>);
        assert!(result1.is_ok());

        // Try to register the second component with the same name from a different path
        let result2 = registry.register_component_from_paths(&file_path_b, None::<&str>);
        assert!(result2.is_err());

        // Verify the error message
        assert!(
            matches!(result2, Err(ParserError::Component(ref msg)) if msg.contains("already registered")),
            "Expected 'already registered' error, got: {:?}",
            result2
        );

        // Verify the original component is still there unchanged
        let component = registry
            .get("my-comp")
            .expect("Failed to retrieve registered component");
        assert_eq!(component.html_content, html_content1);
    }

    #[test]
    fn test_exclude_dot_in_component_name() {
        let mut registry = ComponentRegistry::new();
        let result = registry.register_component("fluent.button", "<p>Dot name</p>", None);

        assert!(
            result.is_err(),
            "Component name with dot but no hyphen should be rejected"
        );
        assert!(
            matches!(result, Err(ParserError::Component(ref msg)) if msg.contains("must contain a hyphen")),
            "Wrong error type or message: {:?}",
            result
        );
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn test_exclude_no_hyphen_html() {
        let mut registry = ComponentRegistry::new();
        let result = registry.register_component("foobar", "<p>No hyphen</p>", None);

        assert!(
            result.is_err(),
            "Component name without hyphen should be rejected"
        );
        assert!(
            matches!(result, Err(ParserError::Component(ref msg)) if msg.contains("must contain a hyphen")),
            "Wrong error type or message: {:?}",
            result
        );
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn test_valid_component_with_hyphen() {
        let mut registry = ComponentRegistry::new();
        let result =
            registry.register_component("fluent-button", "<button>Click me</button>", None);

        assert!(
            result.is_ok(),
            "Component name with hyphen should be accepted"
        );
        assert!(registry.contains("fluent-button"));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn test_valid_component_css_only() {
        let mut registry = ComponentRegistry::new();
        let result =
            registry.register_component("styled-widget", "", Some(".widget { color: blue; }"));

        assert!(
            result.is_ok(),
            "Component with empty HTML and CSS should be accepted"
        );
        assert!(registry.contains("styled-widget"));

        let component = registry
            .get("styled-widget")
            .expect("Failed to retrieve registered component");
        assert_eq!(component.html_content, "");
        assert_eq!(
            component.css_content.as_deref(),
            Some(".widget { color: blue; }")
        );
    }

    #[test]
    fn test_component_name_requires_hyphen() {
        let mut registry = ComponentRegistry::new();
        let result = registry.register_component("single", "<p>Single word</p>", None);

        assert!(
            result.is_err(),
            "Single-word component name should be rejected"
        );
        assert!(
            matches!(result, Err(ParserError::Component(ref msg)) if msg.contains("must contain a hyphen")),
            "Wrong error type or message: {:?}",
            result
        );
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn test_multiple_hyphens_valid() {
        let mut registry = ComponentRegistry::new();
        let result =
            registry.register_component("my-custom-element", "<div>Custom element</div>", None);

        assert!(
            result.is_ok(),
            "Component name with multiple hyphens should be accepted"
        );
        assert!(registry.contains("my-custom-element"));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn test_empty_component_name_rejected() {
        let mut registry = ComponentRegistry::new();
        let result = registry.register_component("", "<p>Empty name</p>", None);

        assert!(result.is_err(), "Empty component name should be rejected");
        assert!(
            matches!(result, Err(ParserError::Component(ref msg)) if msg.contains("must contain a hyphen")),
            "Wrong error type or message: {:?}",
            result
        );
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn test_register_component_extracts_css_tokens() {
        let mut registry = ComponentRegistry::new();
        registry
            .register_component(
                "my-btn",
                "<button>Click</button>",
                Some(":host { color: var(--text-color); padding: var(--spacing-m); }"),
            )
            .expect("register failed");

        let component = registry.get("my-btn").expect("component not found");
        assert_eq!(component.css_tokens, vec!["spacing-m", "text-color"]);
    }

    #[test]
    fn test_register_component_no_css_no_tokens() {
        let mut registry = ComponentRegistry::new();
        registry
            .register_component("my-card", "<div>Card</div>", None)
            .expect("register failed");

        let component = registry.get("my-card").expect("component not found");
        assert!(component.css_tokens.is_empty());
    }

    #[test]
    fn test_register_component_excludes_local_defs_from_tokens() {
        let mut registry = ComponentRegistry::new();
        registry
            .register_component(
                "my-widget",
                "<div>W</div>",
                Some(":host { --local: 5px; margin: var(--external); width: var(--local); }"),
            )
            .expect("register failed");

        let component = registry.get("my-widget").expect("component not found");
        // --local is defined locally so excluded; only --external is a token
        assert_eq!(component.css_tokens, vec!["external"]);
    }
}
