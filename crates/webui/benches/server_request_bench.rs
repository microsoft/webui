// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Benchmarks the high-level Rust server helper for full-document rendering.

use std::collections::HashMap;
use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use webui::server::{serve_request, ServeRequest, ServeResponse};
use webui::{Protocol, RenderOptions, WebUIHandler};
use webui_handler::plugin::webui::WebUIHydrationPlugin;
use webui_protocol::StateProjectionMode;
use webui_protocol::{ComponentData, FragmentList, WebUIFragment, WebUIProtocol};

const ENTRY_ID: &str = "index.html";
const REQUEST_PATH: &str = "/";

fn structural_fragment(value: &str) -> WebUIFragment {
    let mut signal = String::with_capacity("}}}webui:".len() + value.len());
    signal.push_str("}}}webui:");
    signal.push_str(value);
    WebUIFragment::signal(signal, true)
}

fn benchmark_protocol() -> Protocol {
    let fragments = HashMap::from([
        (
            ENTRY_ID.to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<!doctype html><html><head>"),
                    structural_fragment("head_end"),
                    WebUIFragment::raw("</head><body>"),
                    WebUIFragment::component("bench-page"),
                    structural_fragment("body_end"),
                    WebUIFragment::raw("</body></html>"),
                ],
                contains_boundary: false,
            },
        ),
        (
            "bench-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<main>ready</main>")],
                contains_boundary: false,
            },
        ),
    ]);
    let mut protocol = WebUIProtocol::new(fragments);
    protocol.components.insert(
        "bench-page".to_string(),
        ComponentData {
            template_json: r#"{"h":"<main>ready</main>","th":1}"#.to_string(),
            template_functions: "[function(){return true}]".to_string(),
            navigation_keys: vec!["selected".to_string()],
            navigation_mode: Some(StateProjectionMode::Keys as i32),
            ..Default::default()
        },
    );
    Protocol::new(protocol)
}

fn server_request_bench(c: &mut Criterion) {
    let protocol = benchmark_protocol();
    let handler = WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()));
    let html_request = ServeRequest::new(RenderOptions::new(ENTRY_ID, REQUEST_PATH), false, "");
    let nonce_request = ServeRequest::new(
        RenderOptions::new(ENTRY_ID, REQUEST_PATH).with_nonce("request-nonce"),
        false,
        "",
    );
    let partial_request = ServeRequest::new(RenderOptions::new(ENTRY_ID, REQUEST_PATH), true, "");
    let large_state = large_projected_state();

    c.bench_function("server_request/full_html_without_nonce", |b| {
        b.iter(|| {
            let response = serve_request(
                black_box(&protocol),
                black_box(&handler),
                serde_json::Value::Null,
                black_box(&html_request),
            )
            .unwrap_or_else(|error| panic!("server request render failed: {error}"));
            match response {
                ServeResponse::Html(html) => black_box(html.len()),
                ServeResponse::Json(_) => panic!("full-document request returned JSON"),
            }
        });
    });

    c.bench_function("server_request/full_html_with_nonce", |b| {
        b.iter(|| {
            let response = serve_request(
                black_box(&protocol),
                black_box(&handler),
                serde_json::Value::Null,
                black_box(&nonce_request),
            )
            .unwrap_or_else(|error| panic!("server request render failed: {error}"));
            match response {
                ServeResponse::Html(html) => black_box(html.len()),
                ServeResponse::Json(_) => panic!("full-document request returned JSON"),
            }
        });
    });

    c.bench_function("server_request/json_partial", |b| {
        b.iter(|| {
            let response = serve_request(
                black_box(&protocol),
                black_box(&handler),
                serde_json::Value::Null,
                black_box(&partial_request),
            )
            .unwrap_or_else(|error| panic!("server request render failed: {error}"));
            match response {
                ServeResponse::Json(json) => black_box(json.len()),
                ServeResponse::Html(_) => panic!("partial request returned HTML"),
            }
        });
    });

    c.bench_function("server_request/json_partial_large_projected_state", |b| {
        b.iter_batched(
            || large_state.clone(),
            |state| {
                let response = serve_request(
                    black_box(&protocol),
                    black_box(&handler),
                    state,
                    black_box(&partial_request),
                )
                .unwrap_or_else(|error| panic!("server request render failed: {error}"));
                match response {
                    ServeResponse::Json(json) => black_box(json.len()),
                    ServeResponse::Html(_) => panic!("partial request returned HTML"),
                }
            },
            BatchSize::SmallInput,
        );
    });
}

fn large_projected_state() -> serde_json::Value {
    let mut unused = Vec::with_capacity(1_000);
    for index in 0..1_000 {
        unused.push(serde_json::json!({
            "id": index,
            "payload": "x".repeat(128),
        }));
    }
    serde_json::json!({
        "selected": "kept",
        "unused": unused,
    })
}

criterion_group!(benches, server_request_bench);
criterion_main!(benches);
