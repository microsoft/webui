// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(unsafe_code)]

use criterion::{criterion_group, criterion_main, Criterion};
use std::collections::HashMap;
use std::ffi::CString;
use std::hint::black_box;
use webui_ffi::{
    webui_free, webui_handler_create, webui_handler_destroy, webui_handler_render,
    webui_protocol_create, webui_protocol_destroy, webui_protocol_render_partial,
};
use webui_protocol::{
    ComponentData, FragmentList, InitialStateStrategy, StateProjectionMode, WebUIFragment,
    WebUIProtocol,
};

fn build_protocol(component_count: usize) -> Vec<u8> {
    let mut fragments = HashMap::with_capacity(component_count + 1);
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::raw("<main>ready</main>")],
        },
    );

    let mut protocol = WebUIProtocol::new(fragments);
    protocol.initial_state_strategy = InitialStateStrategy::Components as i32;
    protocol.components.reserve(component_count);
    for index in 0..component_count {
        let name = format!("bench-component-{index:04}");
        protocol.fragments.insert(
            name.clone(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<div>component</div>")],
            },
        );
        protocol.components.insert(
            name,
            ComponentData {
                template_json: r#"{"html":"<div>component</div>"}"#.to_string(),
                hydration_mode: StateProjectionMode::Keys as i32,
                hydration_keys: vec!["title".to_string(), "items".to_string()],
                ..Default::default()
            },
        );
    }
    protocol.tokens = (0..128)
        .map(|index| format!("colorBenchToken{index:03}"))
        .collect();
    protocol
        .to_protobuf()
        .unwrap_or_else(|error| panic!("benchmark protocol encode failed: {error}"))
}

fn c_string(value: &str) -> CString {
    CString::new(value)
        .unwrap_or_else(|error| panic!("benchmark string contains an interior NUL: {error}"))
}

fn protocol_bench(c: &mut Criterion) {
    let state = c_string(r#"{"title":"Protocol benchmark","items":[]}"#);
    let entry = c_string("index.html");
    let path = c_string("/");
    let inventory = c_string("");

    let handler = webui_handler_create();
    assert!(!handler.is_null(), "handler creation failed");

    let mut group = c.benchmark_group("ffi_protocol_startup");
    for component_count in [100, 1_000] {
        let protocol = build_protocol(component_count);
        // SAFETY: protocol points to protocol.len() initialized bytes.
        let prepared = unsafe { webui_protocol_create(protocol.as_ptr(), protocol.len()) };
        assert!(!prepared.is_null(), "protocol preparation failed");

        group.bench_function(
            format!("full_prepare_each_render/{component_count}_components"),
            |b| {
                b.iter(|| {
                    // SAFETY: protocol points to protocol.len() initialized bytes.
                    let request_protocol =
                        unsafe { webui_protocol_create(protocol.as_ptr(), protocol.len()) };
                    assert!(!request_protocol.is_null(), "protocol preparation failed");
                    // SAFETY: All opaque and string pointers remain valid for the call.
                    let output = unsafe {
                        webui_handler_render(
                            handler,
                            request_protocol,
                            state.as_ptr(),
                            entry.as_ptr(),
                            path.as_ptr(),
                        )
                    };
                    assert!(!output.is_null(), "full render failed");
                    black_box(output);
                    // SAFETY: output was allocated by webui_handler_render.
                    unsafe { webui_free(output) };
                    // SAFETY: request_protocol is live and no longer borrowed.
                    unsafe { webui_protocol_destroy(request_protocol) };
                });
            },
        );
        group.bench_function(format!("full_reused/{component_count}_components"), |b| {
            b.iter(|| {
                // SAFETY: All opaque and string pointers remain valid for the call.
                let output = unsafe {
                    webui_handler_render(
                        handler,
                        prepared,
                        state.as_ptr(),
                        entry.as_ptr(),
                        path.as_ptr(),
                    )
                };
                assert!(!output.is_null(), "prepared full render failed");
                black_box(output);
                // SAFETY: output was allocated by webui_handler_render.
                unsafe { webui_free(output) };
            });
        });
        group.bench_function(
            format!("partial_prepare_each_render/{component_count}_components"),
            |b| {
                b.iter(|| {
                    // SAFETY: protocol points to protocol.len() initialized bytes.
                    let request_protocol =
                        unsafe { webui_protocol_create(protocol.as_ptr(), protocol.len()) };
                    assert!(!request_protocol.is_null(), "protocol preparation failed");
                    // SAFETY: All opaque and string pointers remain valid for the call.
                    let output = unsafe {
                        webui_protocol_render_partial(
                            request_protocol,
                            state.as_ptr(),
                            entry.as_ptr(),
                            path.as_ptr(),
                            inventory.as_ptr(),
                        )
                    };
                    assert!(!output.is_null(), "partial render failed");
                    black_box(output);
                    // SAFETY: output was allocated by webui_protocol_render_partial.
                    unsafe { webui_free(output) };
                    // SAFETY: request_protocol is live and no longer borrowed.
                    unsafe { webui_protocol_destroy(request_protocol) };
                });
            },
        );
        group.bench_function(
            format!("partial_reused/{component_count}_components"),
            |b| {
                b.iter(|| {
                    // SAFETY: All opaque and string pointers remain valid for the call.
                    let output = unsafe {
                        webui_protocol_render_partial(
                            prepared,
                            state.as_ptr(),
                            entry.as_ptr(),
                            path.as_ptr(),
                            inventory.as_ptr(),
                        )
                    };
                    assert!(!output.is_null(), "prepared partial render failed");
                    black_box(output);
                    // SAFETY: output was allocated by webui_protocol_render_partial.
                    unsafe { webui_free(output) };
                });
            },
        );

        // SAFETY: The prepared handle is live and its benchmark closures are complete.
        unsafe { webui_protocol_destroy(prepared) };
    }
    group.finish();

    // SAFETY: The handler is live and no benchmark closure can use it now.
    unsafe {
        webui_handler_destroy(handler);
    }
}

criterion_group!(benches, protocol_bench);
criterion_main!(benches);
