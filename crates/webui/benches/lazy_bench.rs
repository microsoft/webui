// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use criterion::{criterion_group, criterion_main, Criterion};
use std::fmt::Write as _;
use std::hint::black_box;
use tempfile::TempDir;
use webui::{build, BuildOptions, LazyRequest, Plugin, Protocol, RenderOptions, WebUIHandler};
use webui_handler::plugin::webui::WebUIHydrationPlugin;

webui_handler::define_string_response_writer!(Output, text);

fn protocol(lazy: bool) -> Protocol {
    let directory = TempDir::new().unwrap();
    std::fs::write(
        directory.path().join("index.html"),
        "<html><head></head><body><bench-owner></bench-owner></body></html>",
    )
    .unwrap();
    let mut template = String::with_capacity(20_000);
    template.push_str(if lazy {
        "<h1>Shell</h1><if condition=\"show\" lazy id=\"settings\">"
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

fn lazy_bench(c: &mut Criterion) {
    let eager = protocol(false);
    let lazy = protocol(true);
    let handler = WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()));
    let closed = serde_json::json!({ "show": false, "value": "Fresh" });
    let open = serde_json::json!({ "show": true, "value": "Fresh" });
    let eager_bytes = render(&eager, &closed, &handler).len();
    let lazy_bytes = render(&lazy, &closed, &handler).len();
    let prepared = lazy
        .prepare_lazy_resources(LazyRequest::new("bench-owner", "settings"))
        .unwrap();
    let request = LazyRequest::new("bench-owner", "settings").with_options(&prepared.options);
    let state = r#"{"value":"Fresh","unused":"not selected"}"#;
    let warm_bytes = lazy.render_lazy_partial_json(state, request).unwrap().len();
    let plain = WebUIHandler::new();
    assert_eq!(
        render(&eager, &closed, &plain),
        render(&lazy, &closed, &plain)
    );
    assert_eq!(render(&eager, &open, &plain), render(&lazy, &open, &plain));
    let warm: serde_json::Value =
        serde_json::from_str(&lazy.render_lazy_partial_json(state, request).unwrap()).unwrap();
    assert_eq!(warm["state"], serde_json::json!({ "value": "Fresh" }));
    assert!(warm.get("templates").is_none());
    assert!(lazy_bytes < eager_bytes / 4);
    eprintln!("lazy fixture bytes: eager_closed={eager_bytes}, lazy_closed={lazy_bytes}, resources={}, warm_state={warm_bytes}", prepared.json.len());

    for (name, protocol, state) in [
        ("closed_eager", &eager, &closed),
        ("closed", &lazy, &closed),
        ("open_eager", &eager, &open),
        ("open", &lazy, &open),
    ] {
        c.bench_function(&format!("lazy/{name}"), |b| {
            b.iter(|| black_box(render(black_box(protocol), black_box(state), &handler)));
        });
    }
    c.bench_function("lazy/resources_cold", |b| {
        b.iter(|| {
            black_box(
                lazy.render_lazy_resources(LazyRequest::new("bench-owner", "settings"))
                    .unwrap(),
            )
        });
    });
    c.bench_function("lazy/state_warm", |b| {
        b.iter(|| {
            black_box(
                lazy.render_lazy_partial_json(black_box(state), request)
                    .unwrap(),
            )
        });
    });
}

criterion_group!(benches, lazy_bench);
criterion_main!(benches);
