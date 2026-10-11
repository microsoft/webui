// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// Calls the C ABI and installs a counting global allocator.
#![allow(unsafe_code)]
#![allow(clippy::disallowed_methods)]

//! Output-buffer reservation tests for the webui-ffi render entry points.
//!
//! These live in their own test binary because they install a global
//! allocator that counts the bytes each thread requests.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::HashMap;
use std::ffi::{CStr, CString};

use webui_ffi::{
    webui_handler_create, webui_handler_destroy, webui_handler_render_result, webui_last_error,
    webui_protocol_create, webui_protocol_destroy, webui_render_result_destroy,
};
use webui_protocol::{FragmentList, WebUIFragment, WebUIProtocol};

thread_local! {
    // Per-thread, so tests running in parallel don't skew each other.
    static REQUESTED_BYTES: Cell<usize> = const { Cell::new(0) };
}

fn count_request(bytes: usize) {
    // `try_with` skips the count during thread teardown instead of panicking.
    let _ = REQUESTED_BYTES.try_with(|total| total.set(total.get().saturating_add(bytes)));
}

fn requested_bytes() -> usize {
    REQUESTED_BYTES.with(Cell::get)
}

struct CountingAllocator;

// SAFETY: Every call forwards unchanged to `System`; counting only touches a
// const-initialized thread-local `Cell`, which never allocates.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count_request(layout.size());
        // SAFETY: The caller upholds `GlobalAlloc::alloc`'s contract.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count_request(layout.size());
        // SAFETY: The caller upholds `GlobalAlloc::alloc_zeroed`'s contract.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count_request(new_size);
        // SAFETY: The caller upholds `GlobalAlloc::realloc`'s contract.
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: The caller upholds `GlobalAlloc::dealloc`'s contract.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

unsafe fn last_error_string() -> String {
    let ptr = webui_last_error();
    if ptr.is_null() {
        "<none>".to_string()
    } else {
        CStr::from_ptr(ptr).to_string_lossy().into_owned()
    }
}

fn build_text_protocol() -> Vec<u8> {
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("<html><body><p>"),
                WebUIFragment::signal("text", false),
                WebUIFragment::raw("</p></body></html>"),
            ],
            contains_boundary: false,
        },
    );
    WebUIProtocol::new(fragments)
        .to_protobuf()
        .expect("serialize test protocol")
}

#[test]
fn unknown_entry_does_not_reserve_previous_render_size() {
    const LARGE_TEXT: usize = 1 << 20;
    let protocol = build_text_protocol();
    let entry = CString::new("index.html").expect("static string");
    let missing = CString::new("missing.html").expect("static string");
    let path = CString::new("/").expect("static string");
    let large_state = CString::new(format!(r#"{{"text":"{}"}}"#, "x".repeat(LARGE_TEXT)))
        .expect("state should not contain NUL");
    let small_state = CString::new(r#"{"text":"x"}"#).expect("static string");

    unsafe {
        let handler = webui_handler_create();
        let prepared = webui_protocol_create(protocol.as_ptr(), protocol.len());
        assert!(!prepared.is_null(), "{}", last_error_string());

        // Two large successes teach the protocol's capacity hint a large size.
        for _ in 0..2 {
            let result = webui_handler_render_result(
                handler,
                prepared,
                large_state.as_ptr(),
                entry.as_ptr(),
                path.as_ptr(),
            );
            assert!(!result.is_null(), "{}", last_error_string());
            webui_render_result_destroy(result);
        }

        // Failures don't update the hint, so every attempt would reserve it.
        for attempt in 0..3 {
            let before = requested_bytes();
            let result = webui_handler_render_result(
                handler,
                prepared,
                small_state.as_ptr(),
                missing.as_ptr(),
                path.as_ptr(),
            );
            let requested = requested_bytes() - before;
            assert!(result.is_null());
            let error = last_error_string();
            assert!(error.contains("missing.html"), "unexpected error: {error}");
            assert!(
                requested < LARGE_TEXT / 16,
                "missing-entry attempt {attempt} requested {requested} bytes"
            );
        }

        // A known entry still renders after the failures.
        let result = webui_handler_render_result(
            handler,
            prepared,
            large_state.as_ptr(),
            entry.as_ptr(),
            path.as_ptr(),
        );
        assert!(!result.is_null(), "{}", last_error_string());
        webui_render_result_destroy(result);

        webui_protocol_destroy(prepared);
        webui_handler_destroy(handler);
    }
}
