// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// FFI tests exercise unsafe C ABI functions.
#![allow(unsafe_code)]
#![allow(clippy::disallowed_methods)]

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
    webui_streaming_session_advance, webui_streaming_session_create,
    webui_streaming_session_destroy, webui_streaming_session_resume, webui_streaming_session_start,
    webui_streaming_session_update, webui_streaming_step_boundary_declaration_id,
    webui_streaming_step_boundary_instance_id, webui_streaming_step_boundary_key_number,
    webui_streaming_step_boundary_key_string, webui_streaming_step_boundary_key_type,
    webui_streaming_step_boundary_name, webui_streaming_step_boundary_owner,
    webui_streaming_step_bytes, webui_streaming_step_destroy, webui_streaming_step_done,
    webui_streaming_step_has_boundary, WEBUI_BOUNDARY_KEY_NONE, WEBUI_BOUNDARY_KEY_NUMBER,
    WEBUI_BOUNDARY_KEY_STRING, WEBUI_BOUNDARY_MODE_FINAL, WEBUI_BOUNDARY_MODE_UPDATABLE,
};
use webui_protocol::{
    FragmentList, InitialStateStrategy, StateProjectionMode, WebUIFragment, WebUIProtocol,
    WebUiFragmentRoute,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn structural_fragment(value: &str) -> WebUIFragment {
    let mut token = String::with_capacity("}}}webui:".len() + value.len());
    token.push_str("}}}webui:");
    token.push_str(value);
    WebUIFragment::signal(token, true)
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
            contains_boundary: false,
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
            contains_boundary: false,
        },
    );
    fragments.insert(
        "mp-category-nav".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::raw("<nav></nav>")],
            contains_boundary: false,
        },
    );
    fragments.insert(
        "mp-search-page".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::component("mp-product-grid")],
            contains_boundary: false,
        },
    );
    fragments.insert(
        "mp-product-grid".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::raw("<grid></grid>")],
            contains_boundary: false,
        },
    );
    fragments.insert(
        "mp-product-page".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::component("mp-product-detail")],
            contains_boundary: false,
        },
    );
    fragments.insert(
        "mp-product-detail".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::raw("<detail></detail>")],
            contains_boundary: false,
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
    search_page.navigation_mode = Some(StateProjectionMode::Keys as i32);
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
            contains_boundary: false,
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
            contains_boundary: false,
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
            contains_boundary: false,
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
                structural_fragment("head_end"),
                WebUIFragment::raw("</head><body>"),
                structural_fragment("body_end"),
                WebUIFragment::raw("</body></html>"),
            ],
            contains_boundary: false,
        },
    );
    let protocol = WebUIProtocol {
        fragments,
        ..Default::default()
    };
    protocol.to_protobuf().expect("serialize test protocol")
}

/// The reserved `$webui` state channel must work end-to-end through the
/// existing `webui_handler_render` symbol: hosts get boundary injection with
/// no new per-string C API, and the reserved key never leaks into the
/// hydration payload.
#[test]
fn state_inject_channel_needs_no_new_render_symbol() {
    let proto_bytes = build_protocol_with_body_end();

    unsafe {
        let handler = webui_handler_create();
        let prepared = prepare_protocol(&proto_bytes);

        let c_json = CString::new(
            r#"{"$webui":{"headEnd":"<meta name='he'>","bodyEnd":"<script>be</script>"}}"#,
        )
        .expect("static string");
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

        let head_end = result.find("<meta name='he'>").expect("headEnd missing");
        let head_close = result.find("</head>").expect("</head> missing");
        let body_end = result.find("<script>be</script>").expect("bodyEnd missing");
        let body_close = result.find("</body>").expect("</body> missing");
        assert!(head_end < head_close, "headEnd misplaced:\n{result}");
        assert!(body_end < body_close, "bodyEnd misplaced:\n{result}");
        assert!(
            !result.contains("$webui"),
            "reserved key must never reach the hydration payload:\n{result}"
        );

        webui_protocol_destroy(prepared);
        webui_handler_destroy(handler);
    }
}

/// A malformed `$webui` value is inert rather than an error, and the
/// reserved key is still stripped from the hydration payload.
#[test]
fn malformed_state_inject_is_inert() {
    let proto_bytes = build_protocol_with_body_end();

    unsafe {
        let handler = webui_handler_create();
        let prepared = prepare_protocol(&proto_bytes);

        let c_json = CString::new(r#"{"$webui":"not-an-object"}"#).expect("static string");
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

        assert!(!result.contains("not-an-object"), "got:\n{result}");
        assert!(!result.contains("$webui"), "got:\n{result}");

        webui_protocol_destroy(prepared);
        webui_handler_destroy(handler);
    }
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
            contains_boundary: false,
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
                structural_fragment("head_end"),
                WebUIFragment::raw("</head><body>"),
                WebUIFragment::component("client-card"),
                structural_fragment("body_end"),
                WebUIFragment::raw("</body></html>"),
            ],
            contains_boundary: false,
        },
    );
    fragments.insert(
        "client-card".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::raw("<p>client</p>")],
            contains_boundary: false,
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
fn handler_render_emits_module_preloads_through_the_c_abi() {
    // The compiler resolves hrefs and puts the finished list in the protocol,
    // so every host gets boundary-aware preloading with no host-side code.
    // This asserts that claim across the FFI boundary, order intact.
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("<html><head>"),
                structural_fragment("head_end"),
                WebUIFragment::raw("</head><body>"),
                structural_fragment("body_end"),
                WebUIFragment::raw("</body></html>"),
            ],
            contains_boundary: false,
        },
    );
    let mut protocol = WebUIProtocol::new(fragments);
    protocol.module_preloads = vec!["/chunk-big.js".to_string(), "/chunk-small.js".to_string()];
    let proto_bytes = protocol.to_protobuf().expect("serialize test protocol");

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
        webui_protocol_destroy(prepared);
        webui_handler_destroy(handler);

        let head_end = result.find("</head>").expect("</head> missing");
        assert!(
            result[..head_end].contains(
                r#"<link rel="modulepreload" href="/chunk-big.js"><link rel="modulepreload" href="/chunk-small.js">"#
            ),
            "module preloads missing or reordered in <head>:\n{result}"
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

// ---------------------------------------------------------------------------
// Tests: host-driven streaming sessions
// ---------------------------------------------------------------------------

/// Build two static sibling declarations with string and number keys.
fn build_streaming_protocol() -> Vec<u8> {
    let fragments = HashMap::from([(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("<!DOCTYPE html><html><head>"),
                structural_fragment("head_start"),
                structural_fragment("head_end"),
                WebUIFragment::raw("</head><body>"),
                structural_fragment("body_start"),
                WebUIFragment::boundary(
                    41,
                    "index.html",
                    "string-row",
                    Some("{{string_key}}".to_string()),
                ),
                WebUIFragment::raw("<p id=\"first\">"),
                WebUIFragment::signal("first_label", false),
                WebUIFragment::raw("/"),
                WebUIFragment::signal("count", false),
                WebUIFragment::raw("</p>"),
                WebUIFragment::boundary_end(41),
                WebUIFragment::raw("<p id=\"between\">between-boundaries</p>"),
                WebUIFragment::boundary(
                    42,
                    "index.html",
                    "number-row",
                    Some("{{number_key}}".to_string()),
                ),
                WebUIFragment::raw("<p id=\"second\">"),
                WebUIFragment::signal("second_label", false),
                WebUIFragment::raw("</p>"),
                WebUIFragment::boundary_end(42),
                WebUIFragment::raw("<p id=\"tail\">tail-after-boundaries</p>"),
                structural_fragment("body_end"),
                WebUIFragment::raw("</body></html>"),
            ],
            contains_boundary: true,
        },
    )]);
    WebUIProtocol::new(fragments)
        .to_protobuf()
        .expect("streaming protocol must serialize")
}

fn build_unkeyed_streaming_protocol() -> Vec<u8> {
    let fragments = HashMap::from([(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("<html><head>"),
                structural_fragment("head_start"),
                structural_fragment("head_end"),
                WebUIFragment::raw("</head><body>"),
                structural_fragment("body_start"),
                WebUIFragment::boundary(9, "index.html", "plain", None),
                WebUIFragment::raw("<p>plain</p>"),
                WebUIFragment::boundary_end(9),
                structural_fragment("body_end"),
                WebUIFragment::raw("</body></html>"),
            ],
            contains_boundary: true,
        },
    )]);
    WebUIProtocol::new(fragments)
        .to_protobuf()
        .expect("unkeyed streaming protocol must serialize")
}

fn build_boundary_free_protocol() -> Vec<u8> {
    WebUIProtocol::new(HashMap::from([(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("<!DOCTYPE html><html><head>"),
                structural_fragment("head_start"),
                structural_fragment("head_end"),
                WebUIFragment::raw("</head><body>"),
                structural_fragment("body_start"),
                WebUIFragment::raw("<p>complete</p>"),
                structural_fragment("body_end"),
                WebUIFragment::raw("</body></html>"),
            ],
            contains_boundary: false,
        },
    )]))
    .to_protobuf()
    .expect("boundary-free protocol must serialize")
}

unsafe fn take_allocated_bytes(ptr: *mut std::os::raw::c_char, len: usize) -> String {
    assert!(
        !ptr.is_null(),
        "streaming call failed: {}",
        last_error_string().unwrap_or_else(|| "<none>".to_string())
    );
    let bytes = std::slice::from_raw_parts(ptr.cast::<u8>(), len);
    let chunk = String::from_utf8(bytes.to_vec()).expect("streaming bytes must be UTF-8");
    webui_free(ptr);
    chunk
}

unsafe fn step_bytes(step: *const c_void) -> String {
    let mut len = usize::MAX;
    let ptr = webui_streaming_step_bytes(step, &mut len);
    assert!(
        !ptr.is_null(),
        "step byte access failed: {}",
        last_error_string().unwrap_or_else(|| "<none>".to_string())
    );
    String::from_utf8(std::slice::from_raw_parts(ptr, len).to_vec())
        .expect("streaming step bytes must be UTF-8")
}

unsafe fn borrowed_step_string(
    step: *const c_void,
    getter: unsafe extern "C" fn(*const c_void, *mut usize) -> *const std::os::raw::c_char,
) -> String {
    let mut len = usize::MAX;
    let ptr = getter(step, &mut len);
    assert!(
        !ptr.is_null(),
        "descriptor string access failed: {}",
        last_error_string().unwrap_or_else(|| "<none>".to_string())
    );
    String::from_utf8(std::slice::from_raw_parts(ptr.cast::<u8>(), len).to_vec())
        .expect("descriptor string must be UTF-8")
}

unsafe fn open_streaming_session(handler: *mut c_void, protocol: *mut c_void) -> *mut c_void {
    let entry = CString::new("index.html").unwrap();
    let path = CString::new("/").unwrap();
    let session = webui_streaming_session_create(handler, protocol, entry.as_ptr(), path.as_ptr());
    assert!(
        !session.is_null(),
        "session creation failed: {}",
        last_error_string().unwrap_or_else(|| "<none>".to_string())
    );
    session
}

#[test]
fn streaming_session_discovers_typed_boundaries_resumes_updates_advances_and_completes() {
    let proto_bytes = build_streaming_protocol();
    unsafe {
        let handler = webui_handler_create_with_plugin(CString::new("webui").unwrap().as_ptr());
        let protocol = prepare_protocol(&proto_bytes);
        let session = open_streaming_session(handler, protocol);
        let state = CString::new(
            r#"{"string_key":"alpha\u0000omega","number_key":7,"first_label":"first","second_label":"second","count":1}"#,
        )
        .unwrap();

        let first_step = webui_streaming_session_start(session, state.as_ptr());
        assert!(!first_step.is_null());
        assert!(!webui_streaming_step_done(first_step));
        assert!(last_error_string().is_none());
        assert!(webui_streaming_step_has_boundary(first_step));

        let mut first_id = u32::MAX;
        let mut declaration_id = u32::MAX;
        let mut key_type = u32::MAX;
        assert!(webui_streaming_step_boundary_instance_id(
            first_step,
            &mut first_id
        ));
        assert!(webui_streaming_step_boundary_declaration_id(
            first_step,
            &mut declaration_id
        ));
        assert!(webui_streaming_step_boundary_key_type(
            first_step,
            &mut key_type
        ));
        assert_eq!(first_id, 0);
        assert_eq!(declaration_id, 41);
        assert_eq!(key_type, WEBUI_BOUNDARY_KEY_STRING);
        assert_eq!(
            borrowed_step_string(first_step, webui_streaming_step_boundary_owner),
            "index.html"
        );
        assert_eq!(
            borrowed_step_string(first_step, webui_streaming_step_boundary_name),
            "string-row"
        );
        assert_eq!(
            borrowed_step_string(first_step, webui_streaming_step_boundary_key_string),
            "alpha\0omega"
        );

        let mut document = step_bytes(first_step);
        assert!(!document.contains("id=\"first\""), "start step: {document}");
        webui_streaming_step_destroy(first_step);

        let first_commit = webui_streaming_session_resume(
            session,
            first_id,
            state.as_ptr(),
            WEBUI_BOUNDARY_MODE_UPDATABLE,
        );
        assert!(!first_commit.is_null());
        assert!(!webui_streaming_step_done(first_commit));
        assert!(!webui_streaming_step_has_boundary(first_commit));
        assert!(last_error_string().is_none());
        let first_bytes = step_bytes(first_commit);
        assert!(
            first_bytes.contains("id=\"first\""),
            "resume step: {first_bytes}"
        );
        assert!(
            first_bytes.contains("first/1"),
            "resume step: {first_bytes}"
        );
        assert!(
            !first_bytes.contains("between-boundaries"),
            "resume included parent bytes: {first_bytes}"
        );
        assert!(
            !first_bytes.contains("id=\"second\""),
            "resume included the next occurrence: {first_bytes}"
        );
        assert!(
            !first_bytes.contains("tail-after-boundaries"),
            "resume included document tail: {first_bytes}"
        );
        document.push_str(&first_bytes);
        webui_streaming_step_destroy(first_commit);

        let patch = CString::new(r#"{"count":2}"#).unwrap();
        let mut untouched_len = usize::MAX;
        assert!(webui_streaming_session_update(
            session,
            first_id,
            std::ptr::null(),
            &mut untouched_len
        )
        .is_null());
        assert_eq!(untouched_len, usize::MAX);
        assert!(last_error_string()
            .unwrap_or_default()
            .contains("patch_json"));
        assert!(webui_streaming_session_update(
            session,
            first_id,
            patch.as_ptr(),
            std::ptr::null_mut()
        )
        .is_null());
        assert!(last_error_string().unwrap_or_default().contains("out_len"));

        let mut update_len = usize::MAX;
        let update =
            webui_streaming_session_update(session, first_id, patch.as_ptr(), &mut update_len);
        let update = take_allocated_bytes(update, update_len);
        assert!(update.contains(r#""count":2"#), "update: {update}");
        document.push_str(&update);

        let second_discovery = webui_streaming_session_advance(session);
        assert!(!second_discovery.is_null());
        assert!(!webui_streaming_step_done(second_discovery));
        assert!(webui_streaming_step_has_boundary(second_discovery));
        assert!(last_error_string().is_none());
        let between_bytes = step_bytes(second_discovery);
        assert!(
            between_bytes.contains("between-boundaries"),
            "advance step: {between_bytes}"
        );
        assert!(
            !between_bytes.contains("id=\"second\""),
            "advance included pending occurrence: {between_bytes}"
        );
        assert!(
            !between_bytes.contains("tail-after-boundaries"),
            "advance included document tail: {between_bytes}"
        );
        document.push_str(&between_bytes);

        let mut second_id = u32::MAX;
        declaration_id = u32::MAX;
        key_type = u32::MAX;
        assert!(webui_streaming_step_boundary_instance_id(
            second_discovery,
            &mut second_id
        ));
        assert!(webui_streaming_step_boundary_declaration_id(
            second_discovery,
            &mut declaration_id
        ));
        assert!(webui_streaming_step_boundary_key_type(
            second_discovery,
            &mut key_type
        ));
        assert_eq!(second_id, 1);
        assert_eq!(declaration_id, 42);
        assert_eq!(key_type, WEBUI_BOUNDARY_KEY_NUMBER);
        assert_eq!(
            borrowed_step_string(second_discovery, webui_streaming_step_boundary_name),
            "number-row"
        );
        let mut number_key = f64::NAN;
        assert!(webui_streaming_step_boundary_key_number(
            second_discovery,
            &mut number_key
        ));
        assert_eq!(number_key, 7.0);
        webui_streaming_step_destroy(second_discovery);

        let second_commit = webui_streaming_session_resume(
            session,
            second_id,
            state.as_ptr(),
            WEBUI_BOUNDARY_MODE_FINAL,
        );
        assert!(!second_commit.is_null());
        assert!(!webui_streaming_step_done(second_commit));
        assert!(!webui_streaming_step_has_boundary(second_commit));
        let second_bytes = step_bytes(second_commit);
        assert!(
            second_bytes.contains("id=\"second\""),
            "resume step: {second_bytes}"
        );
        assert!(
            second_bytes.contains("second"),
            "resume step: {second_bytes}"
        );
        assert!(
            !second_bytes.contains("tail-after-boundaries"),
            "resume included document tail: {second_bytes}"
        );
        assert!(
            !second_bytes.contains("</html>"),
            "resume completed the document: {second_bytes}"
        );
        document.push_str(&second_bytes);
        webui_streaming_step_destroy(second_commit);

        let final_step = webui_streaming_session_advance(session);
        assert!(!final_step.is_null());
        assert!(webui_streaming_step_done(final_step));
        assert!(!webui_streaming_step_has_boundary(final_step));
        assert!(last_error_string().is_none());
        let tail_bytes = step_bytes(final_step);
        assert!(
            tail_bytes.contains("tail-after-boundaries"),
            "final advance step: {tail_bytes}"
        );
        assert!(
            tail_bytes.contains("</html>"),
            "final advance step: {tail_bytes}"
        );
        document.push_str(&tail_bytes);
        webui_streaming_step_destroy(final_step);

        assert!(document.starts_with("<!DOCTYPE html>"));
        assert!(document.contains("first"));
        assert!(document.contains("second"));
        assert!(document.contains("</html>"));

        webui_streaming_session_destroy(session);
        webui_protocol_destroy(protocol);
        webui_handler_destroy(handler);
    }
}

#[test]
fn streaming_step_accessors_reject_null_output_pointers() {
    let proto_bytes = build_unkeyed_streaming_protocol();
    unsafe {
        let handler = webui_handler_create();
        let protocol = prepare_protocol(&proto_bytes);
        let session = open_streaming_session(handler, protocol);

        let empty = CString::new("{}").unwrap();
        let step = webui_streaming_session_start(session, empty.as_ptr());
        assert!(!step.is_null());

        assert!(webui_streaming_step_bytes(step, std::ptr::null_mut()).is_null());
        assert!(last_error_string().unwrap_or_default().contains("out_len"));
        assert!(webui_streaming_step_boundary_owner(step, std::ptr::null_mut()).is_null());
        assert!(last_error_string().unwrap_or_default().contains("out_len"));

        webui_streaming_step_destroy(step);

        webui_streaming_session_destroy(session);
        webui_protocol_destroy(protocol);
        webui_handler_destroy(handler);
    }
}

#[test]
fn streaming_step_reports_an_unkeyed_boundary_unambiguously() {
    let proto_bytes = build_unkeyed_streaming_protocol();
    unsafe {
        let handler = webui_handler_create();
        let protocol = prepare_protocol(&proto_bytes);
        let session = open_streaming_session(handler, protocol);

        let empty = CString::new("{}").unwrap();
        let step = webui_streaming_session_start(session, empty.as_ptr());
        assert!(!step.is_null());
        let mut key_type = u32::MAX;
        assert!(webui_streaming_step_boundary_key_type(step, &mut key_type));
        assert_eq!(key_type, WEBUI_BOUNDARY_KEY_NONE);

        let mut len = usize::MAX;
        assert!(webui_streaming_step_boundary_key_string(step, &mut len).is_null());
        assert_eq!(len, usize::MAX);
        assert!(last_error_string()
            .unwrap_or_default()
            .contains("not a string"));

        let mut number = 12.0;
        assert!(!webui_streaming_step_boundary_key_number(step, &mut number));
        assert_eq!(number, 12.0);
        assert!(last_error_string()
            .unwrap_or_default()
            .contains("not a number"));
        webui_streaming_step_destroy(step);

        webui_streaming_session_destroy(session);
        webui_protocol_destroy(protocol);
        webui_handler_destroy(handler);
    }
}

#[test]
fn streaming_session_rejects_out_of_order_calls_without_poisoning() {
    let proto_bytes = build_unkeyed_streaming_protocol();
    unsafe {
        let handler = webui_handler_create();
        let protocol = prepare_protocol(&proto_bytes);
        let session = open_streaming_session(handler, protocol);

        let empty = CString::new("{}").unwrap();
        let first = webui_streaming_session_start(session, empty.as_ptr());
        assert!(!first.is_null());
        let mut instance_id = u32::MAX;
        assert!(webui_streaming_step_boundary_instance_id(
            first,
            &mut instance_id
        ));
        webui_streaming_step_destroy(first);

        assert!(webui_streaming_session_advance(session).is_null());
        assert!(last_error_string()
            .unwrap_or_default()
            .contains("no committed boundary"));

        assert!(webui_streaming_session_resume(
            session,
            instance_id,
            std::ptr::null(),
            WEBUI_BOUNDARY_MODE_FINAL
        )
        .is_null());
        assert!(last_error_string()
            .unwrap_or_default()
            .contains("state_json"));

        let rejected =
            webui_streaming_session_resume(session, 99, empty.as_ptr(), WEBUI_BOUNDARY_MODE_FINAL);
        assert!(rejected.is_null());
        let error = last_error_string().unwrap_or_default();
        assert!(error.contains("stale"), "error: {error}");

        let commit = webui_streaming_session_resume(
            session,
            instance_id,
            empty.as_ptr(),
            WEBUI_BOUNDARY_MODE_FINAL,
        );
        assert!(!commit.is_null());
        assert!(!webui_streaming_step_done(commit));
        assert!(!webui_streaming_step_has_boundary(commit));
        webui_streaming_step_destroy(commit);

        let final_step = webui_streaming_session_advance(session);
        assert!(!final_step.is_null());
        assert!(webui_streaming_step_done(final_step));
        assert!(!webui_streaming_step_has_boundary(final_step));
        assert!(last_error_string().is_none());
        webui_streaming_step_destroy(final_step);

        let after = webui_streaming_session_start(session, empty.as_ptr());
        assert!(after.is_null());
        let error = last_error_string().unwrap_or_default();
        assert!(
            error.contains("completed") || error.contains("started"),
            "error: {error}"
        );
        assert!(webui_streaming_session_advance(session).is_null());
        assert!(last_error_string()
            .unwrap_or_default()
            .contains("completed"));

        webui_streaming_session_destroy(session);
        webui_protocol_destroy(protocol);
        webui_handler_destroy(handler);
    }
}

#[test]
fn streaming_session_boundary_free_start_completes() {
    let proto_bytes = build_boundary_free_protocol();
    unsafe {
        let handler = webui_handler_create();
        let protocol = prepare_protocol(&proto_bytes);
        let session = open_streaming_session(handler, protocol);

        let empty = CString::new("{}").unwrap();
        let step = webui_streaming_session_start(session, empty.as_ptr());
        assert!(!step.is_null());
        assert!(webui_streaming_step_done(step));
        assert!(!webui_streaming_step_has_boundary(step));
        let html = step_bytes(step);
        assert!(html.contains("<p>complete</p>"));

        let mut untouched = 77u32;
        assert!(!webui_streaming_step_boundary_instance_id(
            step,
            &mut untouched
        ));
        assert_eq!(untouched, 77);
        assert!(last_error_string()
            .unwrap_or_default()
            .contains("no pending boundary"));
        webui_streaming_step_destroy(step);

        webui_streaming_session_destroy(session);
        webui_protocol_destroy(protocol);
        webui_handler_destroy(handler);
    }
}

#[test]
fn streaming_session_rejects_invalid_state_json() {
    let proto_bytes = build_unkeyed_streaming_protocol();
    unsafe {
        let handler = webui_handler_create();
        let protocol = prepare_protocol(&proto_bytes);
        let session = open_streaming_session(handler, protocol);

        assert!(webui_streaming_session_start(session, std::ptr::null()).is_null());
        assert!(last_error_string()
            .unwrap_or_default()
            .contains("state_json"));

        let broken = CString::new("{not json").unwrap();
        let step = webui_streaming_session_start(session, broken.as_ptr());
        assert!(step.is_null());
        let error = last_error_string().unwrap_or_default();
        assert!(error.contains("state_json"), "error: {error}");

        let empty = CString::new("{}").unwrap();
        let recovered = webui_streaming_session_start(session, empty.as_ptr());
        assert!(!recovered.is_null());
        assert!(last_error_string().is_none());
        webui_streaming_step_destroy(recovered);

        webui_streaming_session_destroy(session);
        webui_protocol_destroy(protocol);
        webui_handler_destroy(handler);
    }
}

#[test]
fn streaming_session_null_arguments_are_safe() {
    unsafe {
        let empty = CString::new("{}").unwrap();
        let mut len = usize::MAX;
        let mut value = u32::MAX;

        assert!(webui_streaming_session_create(
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null()
        )
        .is_null());
        assert!(webui_streaming_session_start(std::ptr::null_mut(), empty.as_ptr()).is_null());
        assert!(webui_streaming_session_advance(std::ptr::null_mut()).is_null());
        assert!(last_error_string()
            .unwrap_or_default()
            .contains("session_ptr"));
        assert!(webui_streaming_session_resume(
            std::ptr::null_mut(),
            0,
            empty.as_ptr(),
            WEBUI_BOUNDARY_MODE_FINAL
        )
        .is_null());
        assert!(
            webui_streaming_session_update(std::ptr::null_mut(), 0, empty.as_ptr(), &mut len)
                .is_null()
        );
        assert!(webui_streaming_session_update(
            std::ptr::null_mut(),
            0,
            empty.as_ptr(),
            std::ptr::null_mut()
        )
        .is_null());

        assert!(webui_streaming_step_bytes(std::ptr::null(), &mut len).is_null());
        assert!(!webui_streaming_step_done(std::ptr::null()));
        assert!(!webui_streaming_step_has_boundary(std::ptr::null()));
        assert!(!webui_streaming_step_boundary_instance_id(
            std::ptr::null(),
            &mut value
        ));
        assert!(webui_streaming_step_boundary_name(std::ptr::null(), &mut len).is_null());
        assert!(!webui_streaming_step_boundary_key_type(
            std::ptr::null(),
            &mut value
        ));
        assert!(last_error_string().is_some());

        webui_streaming_session_destroy(std::ptr::null_mut());
        webui_streaming_step_destroy(std::ptr::null_mut());

        let proto_bytes = build_unkeyed_streaming_protocol();
        let handler = webui_handler_create();
        let protocol = prepare_protocol(&proto_bytes);
        let session = open_streaming_session(handler, protocol);
        let step = webui_streaming_session_start(session, empty.as_ptr());
        assert!(!step.is_null());
        let rejected = webui_streaming_session_resume(session, 0, empty.as_ptr(), 99);
        assert!(rejected.is_null());
        assert!(last_error_string()
            .unwrap_or_default()
            .contains("invalid boundary mode"));
        webui_streaming_step_destroy(step);
        webui_streaming_session_destroy(session);
        webui_protocol_destroy(protocol);
        webui_handler_destroy(handler);
    }
}

#[test]
fn streaming_session_applies_the_handler_nonce() {
    let proto_bytes = build_unkeyed_streaming_protocol();
    unsafe {
        let handler = webui_handler_create_with_plugin(CString::new("webui").unwrap().as_ptr());
        webui_handler_set_nonce(handler, CString::new("abc123").unwrap().as_ptr());
        let protocol = prepare_protocol(&proto_bytes);
        let session = open_streaming_session(handler, protocol);

        let empty = CString::new("{}").unwrap();
        let first = webui_streaming_session_start(session, empty.as_ptr());
        assert!(!first.is_null());
        let mut instance_id = u32::MAX;
        assert!(webui_streaming_step_boundary_instance_id(
            first,
            &mut instance_id
        ));
        let mut document = step_bytes(first);
        webui_streaming_step_destroy(first);

        let commit = webui_streaming_session_resume(
            session,
            instance_id,
            empty.as_ptr(),
            WEBUI_BOUNDARY_MODE_FINAL,
        );
        assert!(!commit.is_null());
        assert!(!webui_streaming_step_done(commit));
        assert!(!webui_streaming_step_has_boundary(commit));
        document.push_str(&step_bytes(commit));
        webui_streaming_step_destroy(commit);

        let final_step = webui_streaming_session_advance(session);
        assert!(!final_step.is_null());
        assert!(webui_streaming_step_done(final_step));
        document.push_str(&step_bytes(final_step));
        webui_streaming_step_destroy(final_step);

        assert!(document.contains("abc123"), "document: {document}");

        webui_streaming_session_destroy(session);
        webui_protocol_destroy(protocol);
        webui_handler_destroy(handler);
    }
}
