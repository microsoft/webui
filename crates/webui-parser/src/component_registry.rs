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
#[cfg(feature = "fs")]
use walkdir::WalkDir;

type ProcessedCss = (String, Vec<String>, Vec<CssFallbackChain>);

/// Represents a web component in the registry.
#[derive(Debug, Clone)]
pub struct Component {
    /// The custom element tag name (e.g., "hello-world")
    pub tag_name: String,

    /// The HTML content of the component
    pub html_content: String,

    /// The CSS content of the component, if any
    pub css_content: Option<String>,

    /// CSS custom property definitions from this component's CSS.
    pub css_definitions: Vec<String>,

    /// CSS `var()` fallback chains from this component's CSS.
    pub css_fallback_chains: Vec<CssFallbackChain>,

    /// Whether this component has an authored client script.
    pub has_script: bool,

    /// Sorted, deduplicated `@observable`/`@attr` property names scanned from
    /// the component's authored client script. This is the component's hydration
    /// surface: the fields the client restores from the bootstrap state. It is
    /// unioned with the template's reactive roots to form the projection
    /// allowlist that shrinks the emitted SSR state. Empty when the component
    /// ships no local script (e.g. npm-provided components).
    pub hydration_attrs: Vec<String>,
}

/// Inputs for registering a component from content strings.
///
/// Grouping the fields keeps [`ComponentRegistry::register_component`] a
/// single-argument call and lets it grow new build-time metadata (such as the
/// hydration surface) without changing its arity. Callers with no hydration
/// surface use [`ComponentRegistration::new`], which defaults it to empty.
#[derive(Debug, Clone)]
pub struct ComponentRegistration<'a> {
    /// The custom element tag name (must contain a hyphen).
    pub tag_name: &'a str,
    /// The component's HTML template content.
    pub html_content: &'a str,
    /// The component's CSS content, if any.
    pub css_content: Option<&'a str>,
    /// Whether authored browser code owns this custom element tag.
    pub has_script: bool,
    /// Scanned `@observable`/`@attr` property names forming the hydration
    /// surface. Empty for components with no scannable client script.
    pub hydration_attrs: Vec<String>,
}

impl<'a> ComponentRegistration<'a> {
    /// Create a registration with an empty hydration surface.
    ///
    /// Convenience for callers with no scannable client script (tests,
    /// npm-provided components, and hosts that hydrate purely from template
    /// roots). Populate [`Self::hydration_attrs`] directly when the component's
    /// authored script has been scanned with [`crate::scan_hydration_attributes`].
    #[must_use]
    pub fn new(
        tag_name: &'a str,
        html_content: &'a str,
        css_content: Option<&'a str>,
        has_script: bool,
    ) -> Self {
        Self {
            tag_name,
            html_content,
            css_content,
            has_script,
            hydration_attrs: Vec::new(),
        }
    }
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
/// Read a component's authored browser module source, if present.
///
/// Prefers `.ts` over `.js`; returns `None` when neither sibling exists. The
/// returned source is scanned for `@observable`/`@attr` decorators to derive the
/// component's build-time hydration surface, and its presence doubles as the
/// static-host `has_script` signal.
fn read_component_script(html_path: &Path) -> Option<String> {
    for ext in ["ts", "js"] {
        let candidate = html_path.with_extension(ext);
        if candidate.exists() {
            if let Ok(source) = fs::read_to_string(&candidate) {
                return Some(source);
            }
        }
    }
    None
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

        // Read CSS content and extract definitions/fallback requirements if available
        let (css_content, css_definitions, css_fallback_chains) = if let Some(css_path) = css_path {
            let css_path = css_path.as_ref();
            if css_path.exists() {
                let content = fs::read_to_string(css_path).map_err(|source| ParserError::IO {
                    context: format!("Failed to read CSS file: {}", css_path.display()),
                    source,
                })?;
                let (content, definitions, requirements) = self.process_css_content(&content)?;
                (Some(content), definitions, requirements)
            } else {
                (None, Vec::new(), Vec::new())
            }
        } else {
            (None, Vec::new(), Vec::new())
        };

        // Read the sibling client module once: its presence is the static-host
        // signal and its source yields the hydration surface.
        let script = read_component_script(html_path);
        let has_script = script.is_some();
        let hydration_attrs = script
            .map(|source| crate::hydration::scan_hydration_attributes(&source))
            .unwrap_or_default();

        // Create and register the component
        let component = Component {
            tag_name: tag_name.to_string(),
            html_content,
            css_content,
            css_definitions,
            css_fallback_chains,
            has_script,
            hydration_attrs,
        };

        self.components.insert(tag_name.to_string(), component);
        Ok(())
    }

    /// Register a component directly from provided content strings.
    ///
    /// The [`ComponentRegistration::hydration_attrs`] field carries the sorted
    /// set of `@observable`/`@attr` property names scanned from the component's
    /// authored client script (see [`crate::scan_hydration_attributes`]). It is
    /// unioned at build time with the template's reactive roots to form the
    /// projection allowlist the handler uses to shrink the emitted bootstrap
    /// state. Use [`ComponentRegistration::new`] when there is no hydration
    /// surface to supply.
    pub fn register_component(&mut self, registration: ComponentRegistration<'_>) -> Result<()> {
        let ComponentRegistration {
            tag_name,
            html_content,
            css_content,
            has_script,
            hydration_attrs,
        } = registration;

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

        // Extract CSS definitions/fallback requirements if CSS content is provided
        let (css_content, css_definitions, css_fallback_chains) = match css_content {
            Some(css) => {
                let (content, definitions, requirements) = self.process_css_content(css)?;
                (Some(content), definitions, requirements)
            }
            None => (None, Vec::new(), Vec::new()),
        };

        let component: Component = Component {
            tag_name: tag_name.to_string(),
            html_content: html_content.to_string(),
            css_content,
            css_definitions,
            css_fallback_chains,
            has_script,
            hydration_attrs,
        };

        // Register the component
        self.components.insert(tag_name.to_string(), component);
        Ok(())
    }

    /// Strip comments and extract CSS definitions/fallback requirements.
    fn process_css_content(&mut self, css_content: &str) -> Result<ProcessedCss> {
        let (_tokens, definitions, requirements, stripped) = self
            .css_parser
            .extract_tokens_definitions_requirements_and_strip_comments(
                css_content,
                self.legal_comments,
            )?;
        let mut sorted_definitions: Vec<String> = definitions.into_iter().collect();
        sorted_definitions.sort();
        Ok((stripped.into_owned(), sorted_definitions, requirements))
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

    /// Check if the registry has no registered components.
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
    fn test_register_component_detects_ts_sibling_script() {
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
    fn test_register_component_detects_js_sibling_script() {
        let mut fs = TestFileSystem::new();
        let html_path = fs.add_file("components/scripted-card.html", "<p>Scripted</p>");
        std::fs::write(html_path.with_extension("js"), "export {};")
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
    fn test_register_component_ignores_tsx_sibling_script() {
        let mut fs = TestFileSystem::new();
        let html_path = fs.add_file("components/scripted-card.html", "<p>Scripted</p>");
        std::fs::write(html_path.with_extension("tsx"), "export {};")
            .expect("Failed to write sibling script");

        let mut registry = ComponentRegistry::new();
        registry
            .register_component_from_paths(&html_path, None::<&str>)
            .expect("register failed");

        let component = registry
            .get("scripted-card")
            .expect("Failed to retrieve registered component");
        assert!(!component.has_script);
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
        let result = registry.register_component(ComponentRegistration::new(
            "string-component",
            html_content,
            Some(css_content),
            true,
        ));

        assert!(result.is_ok());
        assert!(registry.contains("string-component"));

        let component = registry
            .get("string-component")
            .expect("Failed to retrieve registered component");
        assert_eq!(component.html_content, html_content);
        assert_eq!(component.css_content.as_deref(), Some(css_content));
        assert!(component.has_script);
    }

    #[test]
    fn test_register_component_strips_css_comments() {
        let mut registry = ComponentRegistry::new();
        registry
            .register_component(ComponentRegistration::new(
                "style-component",
                "<p>Styled</p>",
                Some("/* var(--ignored) */ p { color: var(--textColor); } /* remove */"),
                true,
            ))
            .expect("register failed");

        let component = registry
            .get("style-component")
            .expect("Failed to retrieve registered component");
        assert_eq!(
            component.css_content.as_deref(),
            Some(" p { color: var(--textColor); } ")
        );
        assert_eq!(component.css_fallback_chains.len(), 1);
    }

    #[test]
    fn test_register_component_preserves_legal_css_comments_by_default() {
        let mut registry = ComponentRegistry::new();
        registry
            .register_component(ComponentRegistration::new(
                "legal-component",
                "<p>Styled</p>",
                Some("/*! @license MIT */ p { color: red; } /* remove */"),
                true,
            ))
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
            .register_component(ComponentRegistration::new(
                "legal-component",
                "<p>Styled</p>",
                Some("/*! @license MIT */ p { color: red; }"),
                true,
            ))
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
        let result = registry.register_component(ComponentRegistration::new(
            "invalid",
            html_content,
            None,
            true,
        ));

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
        let result1 = registry.register_component(ComponentRegistration::new(
            "dupe-component",
            html_content,
            None,
            true,
        ));
        assert!(result1.is_ok());

        // Try to register a second component with the same name
        let result2 = registry.register_component(ComponentRegistration::new(
            "dupe-component",
            html_content2,
            None,
            true,
        ));
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
        let result = registry.register_component(ComponentRegistration::new(
            "fluent.button",
            "<p>Dot name</p>",
            None,
            true,
        ));

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
        let result = registry.register_component(ComponentRegistration::new(
            "foobar",
            "<p>No hyphen</p>",
            None,
            true,
        ));

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
        let result = registry.register_component(ComponentRegistration::new(
            "fluent-button",
            "<button>Click me</button>",
            None,
            true,
        ));

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
        let result = registry.register_component(ComponentRegistration::new(
            "styled-widget",
            "",
            Some(".widget { color: blue; }"),
            true,
        ));

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
        let result = registry.register_component(ComponentRegistration::new(
            "single",
            "<p>Single word</p>",
            None,
            true,
        ));

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
        let result = registry.register_component(ComponentRegistration::new(
            "my-custom-element",
            "<div>Custom element</div>",
            None,
            true,
        ));

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
        let result = registry.register_component(ComponentRegistration::new(
            "",
            "<p>Empty name</p>",
            None,
            true,
        ));

        assert!(result.is_err(), "Empty component name should be rejected");
        assert!(
            matches!(result, Err(ParserError::Component(ref msg)) if msg.contains("must contain a hyphen")),
            "Wrong error type or message: {:?}",
            result
        );
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn test_register_component_extracts_css_fallback_requirements() {
        let mut registry = ComponentRegistry::new();
        registry
            .register_component(ComponentRegistration::new(
                "my-btn",
                "<button>Click</button>",
                Some(":host { color: var(--text-color); padding: var(--spacing-m); }"),
                true,
            ))
            .expect("register failed");

        let component = registry.get("my-btn").expect("component not found");
        assert_eq!(component.css_fallback_chains.len(), 2);
    }

    #[test]
    fn test_register_component_no_css_no_requirements() {
        let mut registry = ComponentRegistry::new();
        registry
            .register_component(ComponentRegistration::new(
                "my-card",
                "<div>Card</div>",
                None,
                true,
            ))
            .expect("register failed");

        let component = registry.get("my-card").expect("component not found");
        assert!(component.css_fallback_chains.is_empty());
    }

    #[test]
    fn test_register_component_tracks_css_fallback_requirements() {
        let mut registry = ComponentRegistry::new();
        registry
            .register_component(ComponentRegistration::new(
                "my-widget",
                "<div>W</div>",
                Some(":host { --local: 5px; margin: var(--external); width: var(--local); }"),
                true,
            ))
            .expect("register failed");

        let component = registry.get("my-widget").expect("component not found");
        assert_eq!(component.css_fallback_chains.len(), 2);
    }
}
