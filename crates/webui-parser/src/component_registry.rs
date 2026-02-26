//! Component registry for WebUI framework.
//!
//! This module manages the registry of web components used in the application.

use crate::{ParserError, Result};
use std::collections::HashMap;
#[cfg(feature = "fs")]
use std::fs;
#[cfg(feature = "fs")]
use std::path::Path;
use std::path::PathBuf;
#[cfg(feature = "fs")]
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
#[path = "component_registry_tests.rs"]
mod component_registry_tests;
