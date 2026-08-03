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
    webui_handler_render, webui_handler_set_nonce, webui_handler_set_state_inject,
    webui_last_error, webui_protocol_create, webui_protocol_destroy, webui_protocol_render_partial,
    webui_protocol_tokens, webui_streaming_session_boundary,
    webui_streaming_session_boundary_count, webui_streaming_session_create,
    webui_streaming_session_destroy, webui_streaming_session_finish,
    webui_streaming_session_is_finished, webui_streaming_session_update,
    webui_streaming_session_write_boundary, webui_streaming_session_write_shell,
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
                structural_fragment("head_end"),
                WebUIFragment::raw("</head><body>"),
                structural_fragment("body_end"),
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

        webui_handler_set_state_inject(handler, true);

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

/// Without the explicit opt-in, a `$webui` object in the state is inert.
#[test]
fn state_inject_is_disabled_by_default() {
    let proto_bytes = build_protocol_with_body_end();

    unsafe {
        let handler = webui_handler_create();
        let prepared = prepare_protocol(&proto_bytes);

        let c_json =
            CString::new(r#"{"$webui":{"bodyEnd":"<script>be</script>"}}"#).expect("static string");
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

        assert!(!result.contains("<script>be</script>"), "got:\n{result}");
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

/// Build a two-boundary streaming entry with one hydrating component each.
fn build_streaming_protocol() -> Vec<u8> {
    let hosts = ["comp-a", "comp-b"];
    let mut entry = vec![
        WebUIFragment::raw("<!DOCTYPE html><html><head>"),
        structural_fragment("head_start"),
        structural_fragment("head_end"),
        WebUIFragment::raw("</head><body>"),
        structural_fragment("body_start"),
    ];
    for (sequence, host) in hosts.iter().enumerate() {
        entry.push(structural_fragment(&format!("boundary_start:{sequence}")));
        entry.push(WebUIFragment::raw(format!("<{host}")));
        entry.push(structural_fragment(&format!("streaming_root:{host}")));
        entry.push(WebUIFragment::raw(">"));
        entry.push(WebUIFragment::component(*host));
        entry.push(WebUIFragment::raw(format!("</{host}>")));
        entry.push(structural_fragment(&format!("boundary_end:{sequence}")));
    }
    entry.push(structural_fragment("body_end"));
    entry.push(WebUIFragment::raw("</body></html>"));

    let mut fragments = HashMap::new();
    fragments.insert("index.html".to_string(), FragmentList { fragments: entry });
    for host in hosts {
        fragments.insert(
            host.to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw(format!("<b>{host}</b>"))],
            },
        );
    }

    let mut document = WebUIProtocol::new(fragments);
    document.initial_state_strategy = InitialStateStrategy::Components as i32;
    document.streaming_boundaries.insert(
        "index.html".to_string(),
        webui_protocol::StreamingBoundaryList {
            names: vec!["first".to_string(), "second".to_string()],
        },
    );
    for (host, key) in hosts.iter().zip(["a_count", "b_count"]) {
        document.components.insert(
            (*host).to_string(),
            webui_protocol::ComponentData {
                template_json: format!(r#"{{"h":"<i>{host}</i>","th":1}}"#),
                hydration_mode: StateProjectionMode::Keys as i32,
                hydration_keys: vec![key.to_string()],
                ..Default::default()
            },
        );
    }
    document
        .to_protobuf()
        .expect("streaming protocol must serialize")
}

/// Drive one chunk-producing call and return its bytes plus the reported length.
unsafe fn take_chunk(ptr: *mut std::os::raw::c_char, len: usize) -> String {
    assert!(
        !ptr.is_null(),
        "streaming call failed: {}",
        last_error_string().unwrap_or_else(|| "<none>".to_string())
    );
    let chunk = CStr::from_ptr(ptr).to_string_lossy().into_owned();
    assert_eq!(chunk.len(), len, "out_len must match the chunk byte length");
    webui_free(ptr);
    chunk
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
fn streaming_session_returns_one_chunk_per_call_through_the_c_abi() {
    let proto_bytes = build_streaming_protocol();
    unsafe {
        let handler = webui_handler_create_with_plugin(CString::new("webui").unwrap().as_ptr());
        let protocol = prepare_protocol(&proto_bytes);
        let session = open_streaming_session(handler, protocol);

        assert_eq!(webui_streaming_session_boundary_count(session), 2);
        assert!(!webui_streaming_session_is_finished(session));

        let mut first = u32::MAX;
        let mut second = u32::MAX;
        assert!(webui_streaming_session_boundary(
            session,
            CString::new("first").unwrap().as_ptr(),
            &mut first
        ));
        assert!(webui_streaming_session_boundary(
            session,
            CString::new("second").unwrap().as_ptr(),
            &mut second
        ));
        assert_eq!(first, 0);
        assert_eq!(second, 1);

        let empty = CString::new("{}").unwrap();
        let a_state = CString::new(r#"{"a_count":1}"#).unwrap();
        let a_patch = CString::new(r#"{"a_count":7}"#).unwrap();
        let b_state = CString::new(r#"{"b_count":2}"#).unwrap();

        let mut document = String::new();
        let mut len = 0usize;
        document.push_str(&take_chunk(
            webui_streaming_session_write_shell(session, empty.as_ptr(), &mut len),
            len,
        ));
        document.push_str(&take_chunk(
            webui_streaming_session_write_boundary(
                session,
                first,
                a_state.as_ptr(),
                true,
                &mut len,
            ),
            len,
        ));
        document.push_str(&take_chunk(
            webui_streaming_session_update(session, first, a_patch.as_ptr(), &mut len),
            len,
        ));
        document.push_str(&take_chunk(
            webui_streaming_session_write_boundary(
                session,
                second,
                b_state.as_ptr(),
                false,
                &mut len,
            ),
            len,
        ));
        document.push_str(&take_chunk(
            webui_streaming_session_finish(session, empty.as_ptr(), &mut len),
            len,
        ));

        assert!(webui_streaming_session_is_finished(session));
        assert!(document.starts_with("<!DOCTYPE html>"));
        assert!(document.contains("<b>comp-a</b>"));
        assert!(document.contains("<b>comp-b</b>"));
        assert!(document.contains(r#"{"a_count":7}"#));
        assert!(document.ends_with("</html>"));

        webui_streaming_session_destroy(session);
        webui_protocol_destroy(protocol);
        webui_handler_destroy(handler);
    }
}

#[test]
fn streaming_session_accepts_a_null_out_len() {
    let proto_bytes = build_streaming_protocol();
    unsafe {
        let handler = webui_handler_create();
        let protocol = prepare_protocol(&proto_bytes);
        let session = open_streaming_session(handler, protocol);

        let empty = CString::new("{}").unwrap();
        let chunk =
            webui_streaming_session_write_shell(session, empty.as_ptr(), std::ptr::null_mut());
        assert!(!chunk.is_null());
        webui_free(chunk);

        webui_streaming_session_destroy(session);
        webui_protocol_destroy(protocol);
        webui_handler_destroy(handler);
    }
}

#[test]
fn streaming_session_reports_an_unknown_boundary_name() {
    let proto_bytes = build_streaming_protocol();
    unsafe {
        let handler = webui_handler_create();
        let protocol = prepare_protocol(&proto_bytes);
        let session = open_streaming_session(handler, protocol);

        let mut boundary = u32::MAX;
        let ok = webui_streaming_session_boundary(
            session,
            CString::new("firts").unwrap().as_ptr(),
            &mut boundary,
        );
        assert!(!ok);
        assert_eq!(boundary, u32::MAX, "out_boundary must stay untouched");
        let error = last_error_string().unwrap_or_default();
        assert!(error.contains("first"), "error: {error}");

        webui_streaming_session_destroy(session);
        webui_protocol_destroy(protocol);
        webui_handler_destroy(handler);
    }
}

#[test]
fn streaming_session_rejects_out_of_order_and_post_finish_calls() {
    let proto_bytes = build_streaming_protocol();
    unsafe {
        let handler = webui_handler_create();
        let protocol = prepare_protocol(&proto_bytes);
        let session = open_streaming_session(handler, protocol);

        let empty = CString::new("{}").unwrap();
        let mut len = 0usize;
        let shell = webui_streaming_session_write_shell(session, empty.as_ptr(), &mut len);
        assert!(!shell.is_null());
        webui_free(shell);

        // Boundary 1 before boundary 0.
        let chunk =
            webui_streaming_session_write_boundary(session, 1, empty.as_ptr(), false, &mut len);
        assert!(chunk.is_null());
        let error = last_error_string().unwrap_or_default();
        assert!(error.contains("order"), "error: {error}");

        // The rejected call wrote nothing, so the response is still usable.
        for boundary in [0u32, 1u32] {
            let chunk = webui_streaming_session_write_boundary(
                session,
                boundary,
                empty.as_ptr(),
                false,
                &mut len,
            );
            assert!(!chunk.is_null());
            webui_free(chunk);
        }
        let tail = webui_streaming_session_finish(session, empty.as_ptr(), &mut len);
        assert!(!tail.is_null());
        webui_free(tail);

        let after = webui_streaming_session_write_shell(session, empty.as_ptr(), &mut len);
        assert!(after.is_null());
        let error = last_error_string().unwrap_or_default();
        assert!(error.contains("already finished"), "error: {error}");

        webui_streaming_session_destroy(session);
        webui_protocol_destroy(protocol);
        webui_handler_destroy(handler);
    }
}

#[test]
fn streaming_session_survives_an_out_of_order_finish() {
    let proto_bytes = build_streaming_protocol();
    unsafe {
        let handler = webui_handler_create();
        let protocol = prepare_protocol(&proto_bytes);
        let session = open_streaming_session(handler, protocol);

        let empty = CString::new("{}").unwrap();
        let mut len = 0usize;
        let shell = webui_streaming_session_write_shell(session, empty.as_ptr(), &mut len);
        assert!(!shell.is_null());
        webui_free(shell);

        let first =
            webui_streaming_session_write_boundary(session, 0, empty.as_ptr(), false, &mut len);
        assert!(!first.is_null());
        webui_free(first);

        // Boundary 1 is still outstanding, so finish is rejected before it
        // writes anything and the open response must survive.
        let rejected = webui_streaming_session_finish(session, empty.as_ptr(), &mut len);
        assert!(rejected.is_null());
        let error = last_error_string().unwrap_or_default();
        assert!(
            error.contains("every boundary must be committed"),
            "error: {error}"
        );
        assert!(!webui_streaming_session_is_finished(session));

        let second =
            webui_streaming_session_write_boundary(session, 1, empty.as_ptr(), false, &mut len);
        assert!(!second.is_null());
        webui_free(second);

        let tail = webui_streaming_session_finish(session, empty.as_ptr(), &mut len);
        assert!(!tail.is_null());
        webui_free(tail);
        assert!(webui_streaming_session_is_finished(session));

        webui_streaming_session_destroy(session);
        webui_protocol_destroy(protocol);
        webui_handler_destroy(handler);
    }
}

#[test]
fn streaming_session_rejects_invalid_state_json() {
    let proto_bytes = build_streaming_protocol();
    unsafe {
        let handler = webui_handler_create();
        let protocol = prepare_protocol(&proto_bytes);
        let session = open_streaming_session(handler, protocol);

        let broken = CString::new("{not json").unwrap();
        let mut len = 0usize;
        let chunk = webui_streaming_session_write_shell(session, broken.as_ptr(), &mut len);
        assert!(chunk.is_null());
        let error = last_error_string().unwrap_or_default();
        assert!(error.contains("state JSON"), "error: {error}");

        webui_streaming_session_destroy(session);
        webui_protocol_destroy(protocol);
        webui_handler_destroy(handler);
    }
}

#[test]
fn streaming_session_null_arguments_are_safe() {
    unsafe {
        let empty = CString::new("{}").unwrap();
        let mut len = 0usize;

        assert!(webui_streaming_session_create(
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null()
        )
        .is_null());
        assert!(webui_streaming_session_write_shell(
            std::ptr::null_mut(),
            empty.as_ptr(),
            &mut len
        )
        .is_null());
        assert!(!webui_streaming_session_boundary(
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null_mut()
        ));
        assert_eq!(webui_streaming_session_boundary_count(std::ptr::null()), 0);
        // A handle that does not exist can never accept another call.
        assert!(webui_streaming_session_is_finished(std::ptr::null()));
        webui_streaming_session_destroy(std::ptr::null_mut());
    }
}

#[test]
fn streaming_session_applies_the_handler_nonce() {
    let proto_bytes = build_streaming_protocol();
    unsafe {
        let handler = webui_handler_create_with_plugin(CString::new("webui").unwrap().as_ptr());
        webui_handler_set_nonce(handler, CString::new("abc123").unwrap().as_ptr());
        let protocol = prepare_protocol(&proto_bytes);
        let session = open_streaming_session(handler, protocol);

        let empty = CString::new("{}").unwrap();
        let state = CString::new(r#"{"a_count":1}"#).unwrap();
        let mut len = 0usize;
        let mut document = take_chunk(
            webui_streaming_session_write_shell(session, empty.as_ptr(), &mut len),
            len,
        );
        document.push_str(&take_chunk(
            webui_streaming_session_write_boundary(session, 0, state.as_ptr(), false, &mut len),
            len,
        ));

        assert!(document.contains("abc123"), "document: {document}");

        webui_streaming_session_destroy(session);
        webui_protocol_destroy(protocol);
        webui_handler_destroy(handler);
    }
}
