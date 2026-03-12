//! Integration tests for the webui-ffi C ABI.
//!
//! These tests call every `#[no_mangle] extern "C"` function through the
//! Rust linkage to verify correctness. The same functions are exported as
//! C symbols for Go, C#, and Python consumers.

use std::ffi::{CStr, CString};

// Re-use the crate's public C API functions directly.
// Because we added "lib" to crate-type, Rust integration tests can link
// against the rlib and call the `pub extern "C"` functions.
use webui_ffi::{
    webui_free, webui_handler_create, webui_handler_destroy, webui_handler_render,
    webui_last_error, webui_render,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Call `webui_render` and return a Rust `String`, freeing the C
/// string afterwards. Panics if the function returns `NULL`.
unsafe fn render(html: &str, json: &str) -> String {
    let c_html = CString::new(html).expect("test html should not contain NUL");
    let c_json = CString::new(json).expect("test json should not contain NUL");
    let ptr = webui_render(c_html.as_ptr(), c_json.as_ptr());
    assert!(
        !ptr.is_null(),
        "webui_render returned NULL; error: {}",
        last_error_string().unwrap_or_else(|| "<none>".to_string())
    );
    let result = CStr::from_ptr(ptr).to_string_lossy().into_owned();
    webui_free(ptr);
    result
}

/// Retrieve the last error as a Rust String, or `None`.
unsafe fn last_error_string() -> Option<String> {
    let ptr = webui_last_error();
    if ptr.is_null() {
        None
    } else {
        Some(CStr::from_ptr(ptr).to_string_lossy().into_owned())
    }
}

// ---------------------------------------------------------------------------
// Tests: webui_render (happy paths)
// ---------------------------------------------------------------------------

#[test]
fn simple_html_passthrough() {
    unsafe {
        let result = render("<p>Hello</p>", "{}");
        assert_eq!(result, "<p>Hello</p>");
    }
}

#[test]
fn signal_substitution() {
    unsafe {
        let result = render("Hello, {{name}}!", r#"{"name":"WebUI"}"#);
        assert_eq!(result, "Hello, WebUI!");
    }
}

#[test]
fn for_loop() {
    unsafe {
        let html = r#"<ul><for each="item in items"><li>{{item}}</li></for></ul>"#;
        let json = r#"{"items":["a","b","c"]}"#;
        let result = render(html, json);
        assert_eq!(result, "<ul><li>a</li><li>b</li><li>c</li></ul>");
    }
}

#[test]
fn if_condition_true() {
    unsafe {
        let html = r#"<if condition="show"><p>Visible</p></if>"#;
        let json = r#"{"show":true}"#;
        let result = render(html, json);
        assert_eq!(result, "<p>Visible</p>");
    }
}

#[test]
fn if_condition_false() {
    unsafe {
        let html = r#"<if condition="show"><p>Hidden</p></if>"#;
        let json = r#"{"show":false}"#;
        let result = render(html, json);
        assert_eq!(result, "");
    }
}

#[test]
fn html_escaping() {
    unsafe {
        let html = "<div>{{content}}</div>";
        let json = r#"{"content":"<script>alert('xss')</script>"}"#;
        let result = render(html, json);
        assert!(
            !result.contains("<script>"),
            "signal output must be HTML-escaped, got: {result}"
        );
        assert!(result.contains("&lt;script&gt;"));
    }
}

#[test]
fn raw_signal_unescaped() {
    unsafe {
        let html = "<div>{{{content}}}</div>";
        let json = r#"{"content":"<b>bold</b>"}"#;
        let result = render(html, json);
        assert_eq!(result, "<div><b>bold</b></div>");
    }
}

#[test]
fn empty_data_object() {
    unsafe {
        let result = render("<p>static</p>", "{}");
        assert_eq!(result, "<p>static</p>");
    }
}

// ---------------------------------------------------------------------------
// Tests: error cases
// ---------------------------------------------------------------------------

#[test]
fn null_html_returns_null_and_sets_error() {
    unsafe {
        let c_json = CString::new("{}").expect("static string");
        let ptr = webui_render(std::ptr::null(), c_json.as_ptr());
        assert!(ptr.is_null());

        let err = last_error_string();
        assert!(
            err.is_some(),
            "expected an error message after NULL html input"
        );
        let msg = err.unwrap_or_default();
        assert!(
            msg.contains("null"),
            "error should mention null, got: {msg}"
        );
    }
}

#[test]
fn null_json_returns_null_and_sets_error() {
    unsafe {
        let c_html = CString::new("<p>hi</p>").expect("static string");
        let ptr = webui_render(c_html.as_ptr(), std::ptr::null());
        assert!(ptr.is_null());

        let err = last_error_string();
        assert!(err.is_some());
    }
}

#[test]
fn invalid_json_returns_null_and_sets_error() {
    unsafe {
        let c_html = CString::new("<p>hi</p>").expect("static string");
        let c_json = CString::new("NOT JSON").expect("static string");
        let ptr = webui_render(c_html.as_ptr(), c_json.as_ptr());
        assert!(ptr.is_null());

        let err = last_error_string().expect("should have error for bad JSON");
        assert!(
            err.contains("JSON"),
            "error should mention JSON, got: {err}"
        );
    }
}

#[test]
fn successful_call_clears_previous_error() {
    unsafe {
        // First, trigger an error
        let c_html = CString::new("<p>hi</p>").expect("static string");
        let c_json = CString::new("NOT JSON").expect("static string");
        let ptr = webui_render(c_html.as_ptr(), c_json.as_ptr());
        assert!(ptr.is_null());
        assert!(last_error_string().is_some(), "error should be set");

        // Now make a successful call
        let result = render("<p>ok</p>", "{}");
        assert_eq!(result, "<p>ok</p>");

        // Error should be cleared
        assert!(
            last_error_string().is_none(),
            "error should be cleared after successful call"
        );
    }
}

// ---------------------------------------------------------------------------
// Tests: handler lifecycle
// ---------------------------------------------------------------------------

#[test]
fn handler_create_and_destroy() {
    unsafe {
        let handler = webui_handler_create();
        assert!(!handler.is_null());
        webui_handler_destroy(handler);
    }
}

#[test]
fn handler_destroy_null_is_safe() {
    unsafe {
        webui_handler_destroy(std::ptr::null_mut()); // should not crash
    }
}

#[test]
fn handler_render_null_args_returns_null() {
    unsafe {
        let handler = webui_handler_create();
        let c_json = CString::new("{}").expect("static string");

        let c_entry = CString::new("index.html").expect("static string");
        let c_request_path = CString::new("/").expect("static string");
        // null protocol data
        let ptr = webui_handler_render(
            handler,
            std::ptr::null(),
            0,
            c_json.as_ptr(),
            c_entry.as_ptr(),
            c_request_path.as_ptr(),
        );
        assert!(ptr.is_null());
        assert!(last_error_string().is_some());

        webui_handler_destroy(handler);
    }
}

// ---------------------------------------------------------------------------
// Tests: fixture file
// ---------------------------------------------------------------------------

#[test]
fn fixture_file_renders_correctly() {
    let html = include_str!("fixtures/simple.html");
    let json = include_str!("fixtures/state.json");
    let expected = include_str!("fixtures/expected_output.html");

    unsafe {
        let result = render(html, json);
        assert_eq!(result, expected);
    }
}

// ---------------------------------------------------------------------------
// Tests: webui_free
// ---------------------------------------------------------------------------

#[test]
fn free_string_null_is_safe() {
    unsafe {
        webui_free(std::ptr::null_mut()); // should not crash
    }
}
