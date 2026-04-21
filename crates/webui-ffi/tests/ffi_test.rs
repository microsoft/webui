// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// FFI tests exercise unsafe C ABI functions.
#![allow(unsafe_code)]

//! Integration tests for the webui-ffi C ABI.
//!
//! These tests call every `#[no_mangle] extern "C"` function through the
//! Rust linkage to verify correctness. The same functions are exported as
//! C symbols for Go, C#, and Python consumers.

use std::collections::HashMap;
use std::ffi::{CStr, CString};

// Re-use the crate's public C API functions directly.
// Because we added "lib" to crate-type, Rust integration tests can link
// against the rlib and call the `pub extern "C"` functions.
use webui_ffi::{
    webui_free, webui_handler_create, webui_handler_destroy, webui_handler_render,
    webui_last_error, webui_render, webui_render_partial,
};
use webui_protocol::{FragmentList, WebUIFragment, WebUIProtocol, WebUiFragmentRoute};

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

#[test]
fn render_partial_returns_templates_inventory_and_chain() {
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::component("mp-app")],
        },
    );
    fragments.insert(
        "mp-app".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::component("mp-category-nav"),
                WebUIFragment::route_from(WebUiFragmentRoute {
                    path: "/search/:category".to_string(),
                    fragment_id: "mp-search-page".to_string(),
                    exact: true,
                    keep_alive: false,
                    ..Default::default()
                }),
                WebUIFragment::route_from(WebUiFragmentRoute {
                    path: "/product/:handle".to_string(),
                    fragment_id: "mp-product-page".to_string(),
                    exact: true,
                    keep_alive: false,
                    ..Default::default()
                }),
            ],
        },
    );
    fragments.insert(
        "mp-category-nav".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::raw("<nav></nav>")],
        },
    );
    fragments.insert(
        "mp-search-page".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::component("mp-product-grid")],
        },
    );
    fragments.insert(
        "mp-product-grid".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::raw("<grid></grid>")],
        },
    );
    fragments.insert(
        "mp-product-page".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::component("mp-product-detail")],
        },
    );
    fragments.insert(
        "mp-product-detail".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::raw("<detail></detail>")],
        },
    );

    let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
    protocol
        .components
        .entry("mp-app".to_string())
        .or_default()
        .template = "<f-template id=app></f-template>".to_string();
    protocol
        .components
        .entry("mp-search-page".to_string())
        .or_default()
        .template = "<f-template id=search></f-template>".to_string();
    protocol
        .components
        .entry("mp-product-grid".to_string())
        .or_default()
        .template = "<f-template id=grid></f-template>".to_string();
    protocol
        .components
        .entry("mp-category-nav".to_string())
        .or_default()
        .template = "<f-template id=nav></f-template>".to_string();

    let protocol_bytes = protocol
        .to_protobuf()
        .expect("protocol should serialize for ffi test");

    unsafe {
        let c_entry = CString::new("index.html").expect("static string");
        let c_state = CString::new(r#"{"query":"shirts"}"#).expect("static string");
        let c_request_path = CString::new("/search/shirts").expect("static string");
        let c_inventory = CString::new("").expect("static string");

        let ptr = webui_render_partial(
            protocol_bytes.as_ptr(),
            protocol_bytes.len(),
            c_state.as_ptr(),
            c_entry.as_ptr(),
            c_request_path.as_ptr(),
            c_inventory.as_ptr(),
        );
        assert!(
            !ptr.is_null(),
            "webui_render_partial returned NULL: {}",
            last_error_string().unwrap_or_else(|| "<none>".to_string())
        );

        let json = CStr::from_ptr(ptr).to_string_lossy().into_owned();
        webui_free(ptr);

        let value: serde_json::Value =
            serde_json::from_str(&json).expect("ffi response should be valid json");

        // State is at top level (caller adds it), not per-entry in chain
        assert!(
            value.get("state").is_some(),
            "partial response should contain top-level 'state' field"
        );
        assert!(value["state"].is_object(), "state should be an object");
        assert_eq!(
            value["state"]["query"].as_str(),
            Some("shirts"),
            "state should contain the passed-in data"
        );

        assert!(
            value.get("templates").is_some(),
            "partial response should contain 'templates' field"
        );
        assert!(
            value["templates"].is_array(),
            "templates should be an array"
        );
        assert!(
            !value["templates"]
                .as_array()
                .expect("templates is array")
                .is_empty(),
            "templates should not be empty for an empty inventory"
        );

        assert!(
            value.get("inventory").is_some(),
            "partial response should contain 'inventory' field"
        );
        assert!(
            value["inventory"].is_string(),
            "inventory should be a string"
        );

        assert!(
            value.get("path").is_some(),
            "partial response should contain 'path' field"
        );
        assert_eq!(
            value["path"].as_str(),
            Some("/search/shirts"),
            "path should match the request path"
        );

        assert!(
            value.get("chain").is_some(),
            "partial response should contain 'chain' field"
        );
        assert!(value["chain"].is_array(), "chain should be an array");
        let chain = value["chain"].as_array().expect("chain should be an array");
        assert!(!chain.is_empty(), "chain should contain at least one entry");

        // Verify chain entry structure
        let first = &chain[0];
        assert!(
            first.get("component").is_some(),
            "chain entry should have 'component' field"
        );
        assert!(
            first.get("path").is_some(),
            "chain entry should have 'path' field"
        );
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
