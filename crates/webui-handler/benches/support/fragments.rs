// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput};
use serde_json::{Map, Value};
use std::hint::black_box;
use webui_handler::{Protocol, RenderOptions, WebUIHandler};

use crate::fragment_support::{compile_document, EvidenceWriter, ENTRY};

const TREE: &str = r#"<body><ul><render fragment="tree" scope="{{items}}" as="nodes"></render></ul>
<fragment name="tree"><for each="node in nodes"><li><span>{{node.label}}</span><if condition="node.children.length"><ul><render fragment="tree" scope="{{node.children}}" as="nodes"></render></ul></if></li></for></fragment></body>"#;
const ORDINARY_LIST: &str = r#"<body><ul><for each="node in items"><li><span>{{node.label}}</span></li></for></ul>
</body>"#;

fn node(children: Vec<Value>) -> Value {
    Value::Object(Map::from_iter([
        ("label".to_string(), Value::String("node".to_string())),
        ("children".to_string(), Value::Array(children)),
    ]))
}

fn state_from_items(items: Vec<Value>) -> Value {
    Value::Object(Map::from_iter([("items".to_string(), Value::Array(items))]))
}

fn deep_state(depth: usize) -> Value {
    let mut items = Vec::new();
    for _ in 0..depth {
        items = vec![node(items)];
    }
    state_from_items(items)
}

fn flat_state(width: usize) -> Value {
    let mut items = Vec::with_capacity(width);
    for _ in 0..width {
        items.push(node(Vec::new()));
    }
    state_from_items(items)
}

fn wide_state(width: usize) -> Value {
    let mut children = Vec::with_capacity(width);
    for _ in 0..width {
        children.push(node(Vec::new()));
    }
    state_from_items(vec![node(children)])
}

fn repeated_source(calls: usize) -> String {
    let mut source = String::with_capacity(calls * 72 + 144);
    source.push_str("<body>");
    for _ in 0..calls {
        source.push_str(r#"<render fragment="shared" scope="{{selected}}" as="value"></render>"#);
    }
    source.push_str(r#"<fragment name="shared"><span>{{value.label}}</span></fragment></body>"#);
    source
}

fn selected_state(payload_bytes: usize) -> Value {
    let selected = Value::Object(Map::from_iter([
        ("label".to_string(), Value::String("kept".to_string())),
        (
            "unused".to_string(),
            Value::String("x".repeat(payload_bytes)),
        ),
    ]));
    Value::Object(Map::from_iter([("selected".to_string(), selected)]))
}

fn render(protocol: &Protocol, state: &Value, writer: &mut EvidenceWriter) {
    WebUIHandler::new()
        .render(protocol, state, &RenderOptions::new(ENTRY, "/"), writer)
        .unwrap_or_else(|error| panic!("rendering fragment benchmark failed: {error}"));
}

struct RenderCase<'a> {
    name: String,
    source: &'a str,
    state: Value,
    item_markup: &'static str,
    items: usize,
}

fn bench_render(c: &mut Criterion, case: RenderCase<'_>) {
    let protocol = Protocol::new(compile_document(case.source));
    let mut warmup = EvidenceWriter::new(4096);
    render(&protocol, &case.state, &mut warmup);
    assert_eq!(
        warmup.output.matches(case.item_markup).count(),
        case.items,
        "every selected node must be rendered exactly once"
    );
    let mut writer = EvidenceWriter::new(warmup.output.len() + 64);
    render(&protocol, &case.state, &mut writer);
    assert_eq!(writer.output, warmup.output);
    let (bytes, writes, growths, flushes) = writer.evidence();
    assert_eq!(growths, 0, "timed writer must not reallocate");
    assert_eq!(flushes, 0, "ordinary rendering must not flush");
    println!(
        "fragment_render {}: items={}, output_bytes={bytes}, write_calls={writes}, \
         writer_growths={growths}",
        case.name, case.items,
    );
    let mut group = c.benchmark_group("fragment_render");
    group.throughput(Throughput::Bytes(bytes as u64));
    group.bench_function(case.name.as_str(), |b| {
        b.iter(|| {
            writer.clear();
            render(black_box(&protocol), black_box(&case.state), &mut writer);
            black_box(writer.evidence());
        });
    });
    group.finish();
}

fn bench_tree_shapes(c: &mut Criterion) {
    let control_state = flat_state(1000);
    let mut ordinary = EvidenceWriter::new(32 * 1024);
    let mut fragment = EvidenceWriter::new(32 * 1024);
    render(
        &Protocol::new(compile_document(ORDINARY_LIST)),
        &control_state,
        &mut ordinary,
    );
    render(
        &Protocol::new(compile_document(TREE)),
        &control_state,
        &mut fragment,
    );
    assert_eq!(
        ordinary.output, fragment.output,
        "ordinary control byte parity"
    );
    for (name, source) in [
        ("ordinary_list_1000", ORDINARY_LIST),
        ("fragment_list_1000", TREE),
    ] {
        bench_render(
            c,
            RenderCase {
                name: name.to_string(),
                source,
                state: flat_state(1000),
                item_markup: "<span>node</span>",
                items: 1000,
            },
        );
    }
    for depth in [16usize, 64, 128, 240] {
        bench_render(
            c,
            RenderCase {
                name: format!("deep/{depth}"),
                source: TREE,
                state: deep_state(depth),
                item_markup: "<span>node</span>",
                items: depth,
            },
        );
    }
    for width in [10usize, 100, 1000, 10_000] {
        bench_render(
            c,
            RenderCase {
                name: format!("wide/{width}"),
                source: TREE,
                state: wide_state(width),
                item_markup: "<span>node</span>",
                items: width + 1,
            },
        );
    }
}

fn bench_repeated_inputs(c: &mut Criterion) {
    for calls in [1usize, 16, 256, 1024] {
        let source = repeated_source(calls);
        for payload_bytes in [0usize, 1024 * 1024] {
            bench_render(
                c,
                RenderCase {
                    name: format!("repeated/{calls}/input_bytes_{payload_bytes}"),
                    source: &source,
                    state: selected_state(payload_bytes),
                    item_markup: "<span>kept</span>",
                    items: calls,
                },
            );
        }
    }
}

fn bench_protocol_load(c: &mut Criterion) {
    let mut group = c.benchmark_group("fragment_protocol_load");
    for calls in [1usize, 16, 256, 1024] {
        let document = compile_document(&repeated_source(calls));
        let bytes = document
            .to_protobuf()
            .unwrap_or_else(|error| panic!("encoding fragment protocol failed: {error}"));
        println!(
            "fragment_protocol_load calls={calls}: records={}, protobuf_bytes={}",
            document.fragments.len(),
            bytes.len(),
        );
        group.throughput(Throughput::Elements(calls as u64));
        group.bench_with_input(
            BenchmarkId::new("prepare", calls),
            &document,
            |b, document| {
                b.iter_batched(
                    || document.clone(),
                    |document| black_box(Protocol::new(black_box(document))),
                    BatchSize::SmallInput,
                );
            },
        );
    }
    let document = compile_document(TREE);
    group.throughput(Throughput::Elements(document.fragments.len() as u64));
    group.bench_function("recursive_graph", |b| {
        b.iter_batched(
            || document.clone(),
            |document| black_box(Protocol::new(black_box(document))),
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

pub(super) fn bench_fragments(c: &mut Criterion) {
    bench_protocol_load(c);
    bench_tree_shapes(c);
    bench_repeated_inputs(c);
}
