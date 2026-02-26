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
    let result = registry.register_component("string-component", html_content, Some(css_content));

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
