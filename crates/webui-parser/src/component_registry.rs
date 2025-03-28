//! Component registry for WebUI framework.
//!
//! This module manages the registry of web components used in the application.

use crate::{ParserError, Result};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Represents a web component in the registry.
#[derive(Debug, Clone)]
pub struct Component {
    /// The custom element tag name (e.g., "hello-world")
    pub tag_name: String,

    /// The HTML content of the component
    pub html_content: String,

    /// The CSS content of the component, if any
    pub css_content: Option<String>,

    /// The file path where this component is defined
    pub source_path: PathBuf,

    /// The class name that implements this component (if available)
    pub class_name: Option<String>,
}

/// Registry of web components.
#[derive(Debug, Default)]
pub struct ComponentRegistry {
    /// Map of component tag names to their component data
    components: HashMap<String, Component>,
}

impl ComponentRegistry {
    /// Create a new component registry.
    pub fn new() -> Self {
        Self {
            components: HashMap::new(),
        }
    }

    /// Register multiple components from directories recursively.
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
        let html_content = fs::read_to_string(html_path)
            .map_err(|e| ParserError::IO(format!("Failed to read HTML file: {}", e)))?;

        // Read CSS content if available
        let css_content = if let Some(css_path) = css_path {
            let css_path = css_path.as_ref();
            if css_path.exists() {
                let content = fs::read_to_string(css_path)
                    .map_err(|e| ParserError::IO(format!("Failed to read CSS file: {}", e)))?;
                Some(content)
            } else {
                None
            }
        } else {
            None
        };

        // Create and register the component
        let component = Component {
            tag_name: tag_name.to_string(),
            html_content,
            css_content,
            source_path: html_path.to_path_buf(),
            class_name: None,
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

        // Create component with dummy path since it's coming from string content
        let component: Component = Component {
            tag_name: tag_name.to_string(),
            html_content: html_content.to_string(),
            css_content: css_content.map(ToString::to_string),
            source_path: PathBuf::new(), // Empty path since it's not from a file
            class_name: None,
        };

        // Register the component
        self.components.insert(tag_name.to_string(), component);
        Ok(())
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
}
