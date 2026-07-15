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
use std::ffi::{c_void, CStr, CString};

// Re-use the crate's public C API functions directly.
// Because we added "lib" to crate-type, Rust integration tests can link
// against the rlib and call the `pub extern "C"` functions.
use webui_ffi::{
    webui_free, webui_handler_create, webui_handler_create_with_plugin, webui_handler_destroy,
    webui_handler_render, webui_handler_set_nonce, webui_last_error, webui_protocol_create,
    webui_protocol_destroy, webui_protocol_render_partial, webui_protocol_tokens,
};
use webui_protocol::{
    FragmentList, InitialStateStrategy, StateProjectionMode, WebUIFragment, WebUIProtocol,
    WebUiFragmentRoute,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Retrieve the last error as a Rust String, or `None`.
unsafe fn last_error_string() -> Option<String> {
    let ptr = webui_last_error();
    if ptr.is_null() {
        None
    } else {
        Some(CStr::from_ptr(ptr).to_string_lossy().into_owned())
    }
}

unsafe fn prepare_protocol(bytes: &[u8]) -> *mut c_void {
    let prepared = webui_protocol_create(bytes.as_ptr(), bytes.len());
    assert!(
        !prepared.is_null(),
        "protocol preparation failed: {}",
        last_error_string().unwrap_or_else(|| "<none>".to_string())
    );
    prepared
}

unsafe fn read_protocol_tokens(bytes: &[u8]) -> String {
    let prepared = prepare_protocol(bytes);
    let ptr = webui_protocol_tokens(prepared);
    assert!(!ptr.is_null(), "protocol token extraction failed");
    let tokens = CStr::from_ptr(ptr).to_string_lossy().into_owned();
    webui_free(ptr);
    webui_protocol_destroy(prepared);
    tokens
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
    protocol.initial_state_strategy = InitialStateStrategy::Components as i32;
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
    let search_page = protocol
        .components
        .entry("mp-search-page".to_string())
        .or_default();
    search_page.hydration_mode = StateProjectionMode::Keys as i32;
    search_page.hydration_keys = vec!["query".to_string()];
    search_page.navigation_mode = StateProjectionMode::Keys as i32;
    search_page.navigation_keys = vec!["query".to_string()];
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
        let prepared = prepare_protocol(&protocol_bytes);

        let ptr = webui_protocol_render_partial(
            prepared,
            c_state.as_ptr(),
            c_entry.as_ptr(),
            c_request_path.as_ptr(),
            c_inventory.as_ptr(),
        );
        assert!(
            !ptr.is_null(),
            "webui_protocol_render_partial returned NULL: {}",
            last_error_string().unwrap_or_else(|| "<none>".to_string())
        );

        let json = CStr::from_ptr(ptr).to_string_lossy().into_owned();
        webui_free(ptr);
        webui_protocol_destroy(prepared);

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
            value["templates"].is_object(),
            "templates should be an object"
        );
        assert!(
            !value["templates"]
                .as_object()
                .expect("templates is object")
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
// Tests: webui_free
// ---------------------------------------------------------------------------

#[test]
fn free_string_null_is_safe() {
    unsafe {
        webui_free(std::ptr::null_mut()); // should not crash
    }
}

// ---------------------------------------------------------------------------
// Tests: webui_protocol_tokens
// ---------------------------------------------------------------------------

#[test]
fn protocol_tokens_empty_vec_returns_empty_string() {
    // A protocol needs at least one fragment to produce non-zero protobuf bytes.
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::raw("<p>hello</p>")],
        },
    );
    let protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
    let bytes = protocol.to_protobuf().expect("serialize");
    assert!(
        !bytes.is_empty(),
        "protobuf with a fragment should be non-empty"
    );

    unsafe {
        assert_eq!(read_protocol_tokens(&bytes), "");
    }
}

#[test]
fn protocol_tokens_single_token() {
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::raw("<p>hello</p>")],
        },
    );
    let protocol = WebUIProtocol::with_tokens(fragments, vec!["colorBrandBackground".to_string()]);
    let bytes = protocol.to_protobuf().expect("serialize");

    unsafe {
        assert_eq!(read_protocol_tokens(&bytes), "colorBrandBackground");
    }
}

#[test]
fn protocol_tokens_multiple_tokens_newline_delimited() {
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::raw("<p>hello</p>")],
        },
    );
    let protocol = WebUIProtocol::with_tokens(
        fragments,
        vec![
            "colorBrandBackground".to_string(),
            "fontSizeBase300".to_string(),
            "spacingHorizontalM".to_string(),
        ],
    );
    let bytes = protocol.to_protobuf().expect("serialize");

    unsafe {
        let result = read_protocol_tokens(&bytes);
        assert_eq!(
            result,
            "colorBrandBackground\nfontSizeBase300\nspacingHorizontalM"
        );
    }
}

// ---------------------------------------------------------------------------
// Tests: webui_handler_set_nonce
// ---------------------------------------------------------------------------

/// Build a minimal protocol that will produce a `<script>` tag when rendered.
/// Includes head_end (for nonce meta) and body_end (for consolidated script)
/// signals. Requires a plugin-enabled handler to trigger the body_end path.
fn build_protocol_with_body_end() -> Vec<u8> {
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("<html><head>"),
                WebUIFragment::signal("head_end".to_string(), true),
                WebUIFragment::raw("</head><body>"),
                WebUIFragment::signal("body_end".to_string(), true),
                WebUIFragment::raw("</body></html>"),
            ],
        },
    );
    let protocol = WebUIProtocol {
        fragments,
        ..Default::default()
    };
    protocol.to_protobuf().expect("serialize test protocol")
}

#[test]
fn handler_set_nonce_applies_to_render() {
    let proto_bytes = build_protocol_with_body_end();

    unsafe {
        let plugin_id = CString::new("webui").expect("static string");
        let handler = webui_handler_create_with_plugin(plugin_id.as_ptr());
        let prepared = prepare_protocol(&proto_bytes);

        // Set a nonce
        let nonce_val = CString::new("Ep7tTOr+HyRkByAPXxZ9ag==").expect("static string");
        webui_handler_set_nonce(handler, nonce_val.as_ptr());

        let c_json = CString::new("{}").expect("static string");
        let c_entry = CString::new("index.html").expect("static string");
        let c_path = CString::new("/").expect("static string");

        let ptr = webui_handler_render(
            handler,
            prepared,
            c_json.as_ptr(),
            c_entry.as_ptr(),
            c_path.as_ptr(),
        );
        assert!(
            !ptr.is_null(),
            "render returned NULL: {}",
            last_error_string().unwrap_or_else(|| "<none>".to_string())
        );

        let result = CStr::from_ptr(ptr).to_string_lossy().into_owned();
        webui_free(ptr);

        // Verify the script tag has the nonce attribute
        assert!(
            result.contains(r#"nonce="Ep7tTOr+HyRkByAPXxZ9ag==""#),
            "rendered HTML should contain nonce attribute on <script>, got:\n{result}"
        );

        // Verify the meta tag is emitted for the client router
        assert!(
            result.contains(r#"<meta name="webui-nonce" content="Ep7tTOr+HyRkByAPXxZ9ag==""#),
            "rendered HTML should contain nonce meta tag, got:\n{result}"
        );

        webui_protocol_destroy(prepared);
        webui_handler_destroy(handler);
    }
}

#[test]
fn handler_render_without_nonce_has_no_nonce_attribute() {
    let proto_bytes = build_protocol_with_body_end();

    unsafe {
        let plugin_id = CString::new("webui").expect("static string");
        let handler = webui_handler_create_with_plugin(plugin_id.as_ptr());
        let prepared = prepare_protocol(&proto_bytes);

        let c_json = CString::new("{}").expect("static string");
        let c_entry = CString::new("index.html").expect("static string");
        let c_path = CString::new("/").expect("static string");

        let ptr = webui_handler_render(
            handler,
            prepared,
            c_json.as_ptr(),
            c_entry.as_ptr(),
            c_path.as_ptr(),
        );
        assert!(
            !ptr.is_null(),
            "render returned NULL: {}",
            last_error_string().unwrap_or_else(|| "<none>".to_string())
        );

        let result = CStr::from_ptr(ptr).to_string_lossy().into_owned();
        webui_free(ptr);

        // Script tag should NOT have a nonce attribute
        assert!(
            !result.contains("nonce="),
            "rendered HTML without set_nonce should not have nonce attribute, got:\n{result}"
        );

        // No meta nonce tag either
        assert!(
            !result.contains("webui-nonce"),
            "rendered HTML without set_nonce should not have nonce meta, got:\n{result}"
        );

        webui_protocol_destroy(prepared);
        webui_handler_destroy(handler);
    }
}

#[test]
fn handler_set_nonce_null_clears_nonce() {
    let proto_bytes = build_protocol_with_body_end();

    unsafe {
        let plugin_id = CString::new("webui").expect("static string");
        let handler = webui_handler_create_with_plugin(plugin_id.as_ptr());
        let prepared = prepare_protocol(&proto_bytes);

        // Set a nonce
        let nonce_val = CString::new("test-nonce-123").expect("static string");
        webui_handler_set_nonce(handler, nonce_val.as_ptr());

        // Clear it by passing NULL
        webui_handler_set_nonce(handler, std::ptr::null());

        let c_json = CString::new("{}").expect("static string");
        let c_entry = CString::new("index.html").expect("static string");
        let c_path = CString::new("/").expect("static string");

        let ptr = webui_handler_render(
            handler,
            prepared,
            c_json.as_ptr(),
            c_entry.as_ptr(),
            c_path.as_ptr(),
        );
        assert!(!ptr.is_null());

        let result = CStr::from_ptr(ptr).to_string_lossy().into_owned();
        webui_free(ptr);

        // Nonce should be cleared — no nonce in output
        assert!(
            !result.contains("nonce="),
            "after clearing nonce with NULL, output should not contain nonce, got:\n{result}"
        );

        webui_protocol_destroy(prepared);
        webui_handler_destroy(handler);
    }
}

#[test]
fn handler_set_nonce_null_handler_sets_error() {
    unsafe {
        let nonce_val = CString::new("some-nonce").expect("static string");
        webui_handler_set_nonce(std::ptr::null_mut(), nonce_val.as_ptr());

        let err = last_error_string();
        assert!(err.is_some(), "should set error for null handler_ptr");
        assert!(err.unwrap().contains("null"), "error should mention null");
    }
}

#[test]
fn protocol_tokens_preserves_order_and_duplicates() {
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::raw("<p>hello</p>")],
        },
    );
    let protocol = WebUIProtocol::with_tokens(
        fragments,
        vec!["zeta".to_string(), "alpha".to_string(), "zeta".to_string()],
    );
    let bytes = protocol.to_protobuf().expect("serialize");

    unsafe {
        assert_eq!(read_protocol_tokens(&bytes), "zeta\nalpha\nzeta");
    }
}

#[test]
fn protocol_tokens_null_handle_returns_null() {
    unsafe {
        let ptr = webui_protocol_tokens(std::ptr::null());
        assert!(ptr.is_null());
        let err = last_error_string().expect("error should be set for null input");
        assert!(
            err.contains("null"),
            "error should mention null, got: {err}"
        );
    }
}

#[test]
fn protocol_tokens_zero_length_returns_empty_string() {
    // A non-null pointer with len 0 should decode as an empty protocol (no tokens).
    let dummy: u8 = 0;
    unsafe {
        let prepared = webui_protocol_create(&dummy as *const u8, 0);
        assert!(!prepared.is_null());
        let ptr = webui_protocol_tokens(prepared);
        assert!(
            !ptr.is_null(),
            "zero-length input should succeed, not return null"
        );
        let result = CStr::from_ptr(ptr).to_string_lossy().into_owned();
        assert_eq!(result, "");
        webui_free(ptr);
        webui_protocol_destroy(prepared);
    }
}

#[test]
fn protocol_create_invalid_protobuf_returns_null() {
    let garbage: &[u8] = &[0xFF, 0xFE, 0xFD];
    unsafe {
        let prepared = webui_protocol_create(garbage.as_ptr(), garbage.len());
        assert!(prepared.is_null());
        let err = last_error_string().expect("error should be set for bad protobuf");
        assert!(
            err.contains("protobuf") || err.contains("parse"),
            "error should mention parse failure, got: {err}"
        );
    }
}

// ---------------------------------------------------------------------------
// Tests: projected hydration state through the C ABI render path
// ---------------------------------------------------------------------------

/// Like [`build_protocol_with_body_end`] but attaches a reachable authored
/// component hydration keys, so the emitted state is projected to that set.
fn build_protocol_with_hydration_keys(hydration_keys: &[&str]) -> Vec<u8> {
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("<html><head>"),
                WebUIFragment::signal("head_end".to_string(), true),
                WebUIFragment::raw("</head><body>"),
                WebUIFragment::component("client-card"),
                WebUIFragment::signal("body_end".to_string(), true),
                WebUIFragment::raw("</body></html>"),
            ],
        },
    );
    fragments.insert(
        "client-card".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::raw("<p>client</p>")],
        },
    );
    let mut protocol = WebUIProtocol::new(fragments);
    protocol.initial_state_strategy = InitialStateStrategy::Components as i32;
    protocol.components.insert(
        "client-card".to_string(),
        webui_protocol::ComponentData {
            hydration_mode: StateProjectionMode::Keys as i32,
            hydration_keys: hydration_keys
                .iter()
                .map(|key| (*key).to_string())
                .collect(),
            ..Default::default()
        },
    );
    protocol.to_protobuf().expect("serialize test protocol")
}

#[test]
fn handler_render_projects_state_to_component_hydration_keys() {
    let proto_bytes = build_protocol_with_hydration_keys(&["kept"]);

    unsafe {
        let plugin_id = CString::new("webui").expect("static string");
        let handler = webui_handler_create_with_plugin(plugin_id.as_ptr());
        let prepared = prepare_protocol(&proto_bytes);

        let c_json =
            CString::new(r#"{"kept":"KEPT_VALUE_FFI","dropped":"DROPPED_VALUE_FFI"}"#).unwrap();
        let c_entry = CString::new("index.html").expect("static string");
        let c_path = CString::new("/").expect("static string");

        let ptr = webui_handler_render(
            handler,
            prepared,
            c_json.as_ptr(),
            c_entry.as_ptr(),
            c_path.as_ptr(),
        );
        assert!(
            !ptr.is_null(),
            "render returned NULL: {}",
            last_error_string().unwrap_or_else(|| "<none>".to_string())
        );

        let result = CStr::from_ptr(ptr).to_string_lossy().into_owned();
        webui_free(ptr);
        webui_protocol_destroy(prepared);
        webui_handler_destroy(handler);

        // Only the hydratable key reaches the bootstrap state block...
        assert!(
            result.contains(r#""kept":"KEPT_VALUE_FFI""#),
            "hydratable key missing from bootstrap state:\n{result}"
        );
        // ...the non-hydratable key is projected out entirely.
        assert!(
            !result.contains("DROPPED_VALUE_FFI"),
            "server-only value leaked into render:\n{result}"
        );
        assert!(
            !result.contains("dropped"),
            "server-only key name leaked into render:\n{result}"
        );
    }
}

#[test]
fn protocol_supports_repeated_full_renders() {
    let proto_bytes = build_protocol_with_hydration_keys(&["kept"]);

    unsafe {
        let plugin_id = CString::new("webui").expect("static string");
        let handler = webui_handler_create_with_plugin(plugin_id.as_ptr());
        let prepared = webui_protocol_create(proto_bytes.as_ptr(), proto_bytes.len());
        assert!(
            !prepared.is_null(),
            "protocol preparation failed: {}",
            last_error_string().unwrap_or_else(|| "<none>".to_string())
        );

        let c_entry = CString::new("index.html").expect("static string");
        let c_path = CString::new("/").expect("static string");
        for expected in ["FIRST_LOADED", "SECOND_LOADED"] {
            let state = CString::new(format!(r#"{{"kept":"{expected}","dropped":"SECRET"}}"#))
                .expect("state should not contain NUL");
            let ptr = webui_handler_render(
                handler,
                prepared,
                state.as_ptr(),
                c_entry.as_ptr(),
                c_path.as_ptr(),
            );
            assert!(
                !ptr.is_null(),
                "prepared render failed: {}",
                last_error_string().unwrap_or_else(|| "<none>".to_string())
            );
            let rendered = CStr::from_ptr(ptr).to_string_lossy().into_owned();
            webui_free(ptr);
            assert!(rendered.contains(expected));
            assert!(!rendered.contains("SECRET"));
        }

        webui_protocol_destroy(prepared);
        webui_handler_destroy(handler);
    }
}

#[test]
fn protocol_exposes_tokens() {
    let protocol = WebUIProtocol::with_tokens(
        HashMap::new(),
        vec!["alpha".to_string(), "beta".to_string()],
    );
    let bytes = protocol.to_protobuf().expect("serialize protocol");

    unsafe {
        let prepared = webui_protocol_create(bytes.as_ptr(), bytes.len());
        assert!(!prepared.is_null());

        let ptr = webui_protocol_tokens(prepared);
        assert!(!ptr.is_null());
        let tokens = CStr::from_ptr(ptr).to_string_lossy().into_owned();
        webui_free(ptr);
        webui_protocol_destroy(prepared);

        assert_eq!(tokens, "alpha\nbeta");
    }
}

#[test]
fn protocol_destroy_null_is_safe() {
    unsafe {
        webui_protocol_destroy(std::ptr::null_mut());
    }
}
