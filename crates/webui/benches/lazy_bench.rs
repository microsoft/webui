// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use criterion::{criterion_group, criterion_main, Criterion};
use std::fmt::Write as _;
use std::hint::black_box;
use tempfile::TempDir;
use webui::{build, BuildOptions, BuildResult, Plugin, Protocol, RenderOptions, WebUIHandler};
use webui_handler::plugin::webui::WebUIHydrationPlugin;

webui_handler::define_string_response_writer!(Output, text);

const REGION: &str = "bench-owner::lazy:@0";

fn compile(lazy: bool) -> BuildResult {
    let directory = TempDir::new().unwrap();
    std::fs::write(
        directory.path().join("index.html"),
        "<html><head></head><body><bench-owner></bench-owner></body></html>",
    )
    .unwrap();
    let mut template = String::with_capacity(20_000);
    template.push_str(if lazy {
        "<h1>Shell</h1><if condition=\"show\" lazy>"
    } else {
        "<h1>Shell</h1><if condition=\"show\">"
    });
    for index in 0..250 {
        write!(template, "<label>Setting {index}<input value=\"").unwrap();
        template.push_str("{{value}}\"></label>");
    }
    template.push_str("</if>");
    std::fs::write(directory.path().join("bench-owner.html"), template).unwrap();
    build(BuildOptions {
        app_dir: directory.path().to_owned(),
        plugin: Some(Plugin::WebUI),
        ..BuildOptions::default()
    })
    .unwrap()
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
    let eager = compile(false);
    let lazy = compile(true);
    assert!(eager.lazy_pack_files.is_empty());
    assert_eq!(lazy.lazy_pack_files.len(), 1);
    let pack_bytes = lazy.lazy_pack_files[0].content.len();
    let eager = Protocol::new(eager.protocol);
    let lazy = Protocol::new(lazy.protocol);
    let handler = WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()));
    let closed = serde_json::json!({ "show": false, "value": "Fresh" });
    let open = serde_json::json!({ "show": true, "value": "Fresh" });
    let eager_bytes = render(&eager, &closed, &handler).len();
    let lazy_html = render(&lazy, &closed, &handler);
    let plain = WebUIHandler::new();
    assert_eq!(
        render(&eager, &closed, &plain),
        render(&lazy, &closed, &plain)
    );
    assert_eq!(render(&eager, &open, &plain), render(&lazy, &open, &plain));
    assert!(
        lazy_html.contains("\"lp\":"),
        "closed owner metadata references its pack"
    );
    assert!(lazy_html.len() < eager_bytes / 4);
    eprintln!(
        "lazy fixture bytes: eager_closed={eager_bytes}, lazy_closed={}, pack={pack_bytes}",
        lazy_html.len()
    );

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
    // Build-time only: hosts serve the rendered pack as a static file.
    c.bench_function("lazy/pack_render", |b| {
        b.iter(|| black_box(lazy.render_lazy_pack(black_box(REGION)).unwrap()));
    });
}

criterion_group!(benches, lazy_bench);
criterion_main!(benches);
