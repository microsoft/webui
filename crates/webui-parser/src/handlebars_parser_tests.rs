use super::*;
use webui_protocol::web_ui_fragment::Fragment;

#[test]
fn test_parse_plain_text() {
    let parser = HandlebarsParser::new();
    let result = parser
        .parse("Hello, World!")
        .expect("Failed to parse plain text");

    assert_eq!(result.len(), 1);
    match result[0].fragment.as_ref() {
        Some(Fragment::Raw(raw)) => assert_eq!(raw.value, "Hello, World!"),
        _ => panic!("Expected Raw fragment"),
    }
}

#[test]
fn test_parse_double_brace() {
    let parser = HandlebarsParser::new();
    let result = parser
        .parse("Hello, {{name}}!")
        .expect("Failed to parse double brace syntax");

    assert_eq!(result.len(), 3);

    match result[0].fragment.as_ref() {
        Some(Fragment::Raw(raw)) => assert_eq!(raw.value, "Hello, "),
        _ => panic!("Expected Raw fragment"),
    }

    match result[1].fragment.as_ref() {
        Some(Fragment::Signal(signal)) => {
            assert_eq!(signal.value, "name");
            assert!(!signal.raw);
        }
        _ => panic!("Expected Signal fragment"),
    }

    match result[2].fragment.as_ref() {
        Some(Fragment::Raw(raw)) => assert_eq!(raw.value, "!"),
        _ => panic!("Expected Raw fragment"),
    }
}

#[test]
fn test_parse_triple_brace() {
    let parser = HandlebarsParser::new();
    let result = parser
        .parse("Content: {{{html_content}}}")
        .expect("Failed to parse triple brace syntax");

    assert_eq!(result.len(), 2);

    match result[0].fragment.as_ref() {
        Some(Fragment::Raw(raw)) => assert_eq!(raw.value, "Content: "),
        _ => panic!("Expected Raw fragment"),
    }

    match result[1].fragment.as_ref() {
        Some(Fragment::Signal(signal)) => {
            assert_eq!(signal.value, "html_content");
            assert!(signal.raw);
        }
        _ => panic!("Expected Signal fragment"),
    }
}

#[test]
fn test_mixed_braces() {
    let parser = HandlebarsParser::new();
    let result = parser
        .parse("Hello, {{name}}! {{{html_content}}}")
        .expect("Failed to parse mixed brace syntax");

    assert_eq!(result.len(), 4);

    match result[0].fragment.as_ref() {
        Some(Fragment::Raw(raw)) => assert_eq!(raw.value, "Hello, "),
        _ => panic!("Expected Raw fragment"),
    }

    match result[1].fragment.as_ref() {
        Some(Fragment::Signal(signal)) => {
            assert_eq!(signal.value, "name");
            assert!(!signal.raw);
        }
        _ => panic!("Expected Signal fragment"),
    }

    match result[2].fragment.as_ref() {
        Some(Fragment::Raw(raw)) => assert_eq!(raw.value, "! "),
        _ => panic!("Expected Raw fragment"),
    }

    match result[3].fragment.as_ref() {
        Some(Fragment::Signal(signal)) => {
            assert_eq!(signal.value, "html_content");
            assert!(signal.raw);
        }
        _ => panic!("Expected Signal fragment"),
    }
}

#[test]
fn test_invalid_triple_open() {
    let parser = HandlebarsParser::new();
    let result = parser.parse("{{{invalid}}").expect("parse failed");
    assert_eq!(result.len(), 1);
    match result[0].fragment.as_ref() {
        Some(Fragment::Raw(raw)) => assert_eq!(raw.value, "{{{invalid}}"),
        _ => panic!("Expected Raw fragment"),
    }
}

#[test]
fn test_four_open_braces() {
    let parser = HandlebarsParser::new();
    let result = parser.parse("{{{{invalid}}").expect("parse failed");
    assert_eq!(result.len(), 1);
    match result[0].fragment.as_ref() {
        Some(Fragment::Raw(raw)) => assert_eq!(raw.value, "{{{{invalid}}"),
        _ => panic!("Expected Raw fragment"),
    }
}

#[test]
fn test_five_braces_with_valid_double() {
    let parser = HandlebarsParser::new();
    let result = parser.parse("{{{{{invalid}}").expect("parse failed");
    assert_eq!(result.len(), 2);
    match result[0].fragment.as_ref() {
        Some(Fragment::Raw(raw)) => assert_eq!(raw.value, "{{{"),
        _ => panic!("Expected Raw fragment for prefix"),
    }
    match result[1].fragment.as_ref() {
        Some(Fragment::Signal(s)) => {
            assert_eq!(s.value, "invalid");
            assert!(!s.raw);
        }
        _ => panic!("Expected Signal fragment"),
    }
}
