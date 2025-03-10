//! Component registry for WebUI framework.
//!
//! This module manages the registry of web components used in the application.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::fs;
use walkdir::WalkDir; // added for recursive scanning
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
                if path.extension().map_or(false, |ext| ext == "html") {
                    // Check for a component name (must contain a hyphen)
                    if let Some(filename) = path.file_stem().and_then(|s| s.to_str()) {
                        if filename.contains('-') {
                            // Find associated CSS file
                            let css_path = path.with_extension("css");
                            // Register the component (key is the file name without extension)
                            self.register_component_from_paths(&path, if css_path.exists() { Some(&css_path) } else { None })?;
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
    use tempfile::NamedTempFile;
    
    fn create_test_file(content: &str, extension: &str) -> PathBuf {
        // Create a temporary file
        let file = NamedTempFile::new().unwrap();
        let file_path = file.path().to_path_buf();
        
        // Create the new path with the desired extension
        let dir = file_path.parent().unwrap();
        let file_name = format!("test-file.{}", extension);
        let new_path = dir.join(file_name);
        
        // Persist  file to the new path
        file.persist(&new_path).unwrap();
        
        // Write the content to the persisted file
        let mut file = fs::File::create(&new_path).unwrap();
        file.write_all(content.as_bytes()).unwrap();
        
        new_path
    }
    
    #[test]
    fn test_register_component() {
        let html_content = "<template><p>Hello World</p></template>";
        let css_content = "p { color: red; }";
        
        // Create temporary files
        let html_path = create_test_file(html_content, "html");
        let css_path = create_test_file(css_content, "css");
        
        // Rename the files to have hyphens (valid component names)
        let new_html_path = html_path.with_file_name("test-component.html");
        fs::rename(&html_path, &new_html_path).unwrap();
        
        let new_css_path = css_path.with_file_name("test-component.css");
        fs::rename(&css_path, &new_css_path).unwrap();
        
        // Register the component
        let mut registry = ComponentRegistry::new();
        let result = registry.register_component_from_paths(&new_html_path, Some(&new_css_path));
        
        assert!(result.is_ok());
        assert!(registry.contains("test-component"));
        
        let component = registry.get("test-component").unwrap();
        assert_eq!(component.html_content, html_content);
        assert_eq!(component.css_content.as_deref(), Some(css_content));
        
        // Clean up the files
        let _ = fs::remove_file(new_html_path);
        let _ = fs::remove_file(new_css_path);
    }
    
    #[test]
    fn test_component_name_validation() {
        let html_content = "<p>Invalid</p>";
        
        // Create temporary file with invalid name (no hyphen)
        let html_path = create_test_file(html_content, "html");
        let new_html_path = html_path.with_file_name("invalid.html");
        fs::rename(&html_path, &new_html_path).unwrap();
        
        // Try to register the component
        let mut registry = ComponentRegistry::new();
        let result = registry.register_component_from_paths(&new_html_path, None::<&str>);
        
        assert!(result.is_err());
        assert_eq!(registry.len(), 0);
        
        // Clean up
        let _ = fs::remove_file(new_html_path);
    }
    
    #[test]
    fn test_missing_css_file() {
        let html_content = "<template><p>CSS Optional</p></template>";
        
        // Create temporary HTML file
        let html_path = create_test_file(html_content, "html");
        let new_html_path = html_path.with_file_name("test-component.html");
        fs::rename(&html_path, &new_html_path).unwrap();
        
        // Register with non-existent CSS file
        let mut registry = ComponentRegistry::new();
        let result = registry.register_component_from_paths(&new_html_path, None::<&str>);
        
        assert!(result.is_ok());
        let component = registry.get("test-component").unwrap();
        assert_eq!(component.html_content, html_content);
        assert_eq!(component.css_content, None);
        
        // Clean up
        let _ = fs::remove_file(new_html_path);
    }
}
