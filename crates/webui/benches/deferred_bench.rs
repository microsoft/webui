// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use criterion::{criterion_group, criterion_main, Criterion};
use std::fmt::Write as _;
use std::hint::black_box;
use tempfile::TempDir;
use webui::{build, BuildOptions, DeferredRequest, Plugin, Protocol, RenderOptions, WebUIHandler};
use webui_handler::plugin::webui::WebUIHydrationPlugin;

webui_handler::define_string_response_writer!(Output, text);

fn protocol(deferred: bool) -> Protocol {
    let directory = TempDir::new().unwrap();
    std::fs::write(
        directory.path().join("index.html"),
        "<html><head></head><body><bench-owner></bench-owner></body></html>",
    )
    .unwrap();
    let mut template = String::with_capacity(20_000);
    template.push_str(if deferred {
        "<h1>Shell</h1><if condition=\"show\" defer id=\"settings\">"
    } else {
        "<h1>Shell</h1><if condition=\"show\">"
    });
    for index in 0..250 {
        write!(template, "<label>Setting {index}<input value=\"").unwrap();
        template.push_str("{{value}}\"></label>");
    }
    template.push_str("</if>");
    std::fs::write(directory.path().join("bench-owner.html"), template).unwrap();
    let result = build(BuildOptions {
        app_dir: directory.path().to_owned(),
        plugin: Some(Plugin::WebUI),
        ..BuildOptions::default()
    })
    .unwrap();
    Protocol::new(result.protocol)
}

fn render(protocol: &Protocol, state: &serde_json::Value, handler: &WebUIHandler) -> String {
    let mut writer = Output::with_capacity(4096);
    handler
        .render(
            protocol,
            state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
    writer.text
}

fn deferred_bench(c: &mut Criterion) {
    let eager = protocol(false);
    let deferred = protocol(true);
    let handler = WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()));
    let closed = serde_json::json!({ "show": false, "value": "Fresh" });
    let open = serde_json::json!({ "show": true, "value": "Fresh" });
    let eager_bytes = render(&eager, &closed, &handler).len();
    let deferred_bytes = render(&deferred, &closed, &handler).len();
    let prepared = deferred
        .prepare_deferred_resources(DeferredRequest::new("bench-owner", "settings"))
        .unwrap();
    let request = DeferredRequest::new("bench-owner", "settings").with_options(&prepared.options);
    let state = r#"{"value":"Fresh","unused":"not selected"}"#;
    let warm_bytes = deferred
        .render_deferred_partial_json(state, request)
        .unwrap()
        .len();
    assert!(deferred_bytes < eager_bytes / 4);
    eprintln!("deferred fixture bytes: eager_closed={eager_bytes}, deferred_closed={deferred_bytes}, resources={}, warm_state={warm_bytes}", prepared.json.len());

    for (name, protocol, state) in [
        ("closed_eager", &eager, &closed),
        ("closed", &deferred, &closed),
        ("open", &deferred, &open),
    ] {
        c.bench_function(&format!("deferred/{name}"), |b| {
            b.iter(|| black_box(render(black_box(protocol), black_box(state), &handler)));
        });
    }
    c.bench_function("deferred/resources_cold", |b| {
        b.iter(|| {
            black_box(
                deferred
                    .render_deferred_resources(DeferredRequest::new("bench-owner", "settings"))
                    .unwrap(),
            )
        });
    });
    c.bench_function("deferred/state_warm", |b| {
        b.iter(|| {
            black_box(
                deferred
                    .render_deferred_partial_json(black_box(state), request)
                    .unwrap(),
            )
        });
    });
}

criterion_group!(benches, deferred_bench);
criterion_main!(benches);
