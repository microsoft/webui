//! Component registry for WebUI framework.
//!
//! This module manages the registry of web components used in the application.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::fs;
use walkdir::WalkDir;
use crate::{ParserError, Result};

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
            for entry in WalkDir::new(dir.as_ref()).into_iter().filter_map(|e| e.ok()) {
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
                            self.register_component_from_paths(path, if css_path.exists() { Some(&css_path) } else { None })?;
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
                "Component name '{}' must contain a hyphen", tag_name
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
    use std::io::Write;
    
    fn create_test_file(content: &str, filename: &str) -> PathBuf {
        // Create a temporary directory to hold our files
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join(filename);
        
        // Write the content to the file
        let mut file = fs::File::create(&file_path).unwrap();
        file.write_all(content.as_bytes()).unwrap();
        
        // Return the path, but keep temp_dir alive by leaking it
        // (it will be cleaned up when the test exits)
        std::mem::forget(temp_dir);
        file_path
    }
    
    #[test]
    fn test_register_component() {
        let html_content = "<template><p>Hello World</p></template>";
        let css_content = "p { color: red; }";
        
        // Create temporary files with proper names directly
        let html_path = create_test_file(html_content, "test-component.html");
        let css_path = create_test_file(css_content, "test-component.css");
        
        // Register the component (no rename needed)
        let mut registry = ComponentRegistry::new();
        let result = registry.register_component_from_paths(&html_path, Some(&css_path));
        
        assert!(result.is_ok());
        assert!(registry.contains("test-component"));
        
        let component = registry.get("test-component").unwrap();
        assert_eq!(component.html_content, html_content);
        assert_eq!(component.css_content.as_deref(), Some(css_content));
    }
    
    #[test]
    fn test_component_name_validation() {
        let html_content = "<p>Invalid</p>";
        
        // Create temporary file with invalid name (no hyphen)
        let html_path = create_test_file(html_content, "invalid.html");
        
        // Try to register the component
        let mut registry = ComponentRegistry::new();
        let result = registry.register_component_from_paths(&html_path, None::<&str>);
        
        assert!(result.is_err());
        assert_eq!(registry.len(), 0);
    }
    
    #[test]
    fn test_missing_css_file() {
        let html_content = "<template><p>CSS Optional</p></template>";
        
        // Create temporary HTML file
        let html_path = create_test_file(html_content, "test-component");
        
        // Register with non-existent CSS file
        let mut registry = ComponentRegistry::new();
        let result = registry.register_component_from_paths(&html_path, None::<&str>);
        
        assert!(result.is_ok());
        let component = registry.get("test-component").unwrap();
        assert_eq!(component.html_content, html_content);
        assert_eq!(component.css_content, None);
    }
}
