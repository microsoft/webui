// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use criterion::{BenchmarkId, Criterion, Throughput};
use std::fmt::Write;
use std::hint::black_box;
use webui_parser::plugin::webui::generate_compiled_template;
use webui_parser::HtmlParser;
use webui_protocol::{web_ui_fragment::Fragment, WebUIProtocol};

const BODY_MARKER: &str = "data-fragment-body";

fn repeated_calls(calls: usize) -> String {
    let mut source = String::with_capacity(calls * 72 + 160);
    for _ in 0..calls {
        source.push_str(r#"<render fragment="shared" scope="{{selected}}" as="value"></render>"#);
    }
    source.push_str(
        r#"<fragment name="shared"><span data-fragment-body="shared">{{value.label}}</span></fragment>"#,
    );
    source
}

fn declaration_chain(declarations: usize) -> String {
    let mut source = String::with_capacity(declarations * 112 + 40);
    source.push_str(r#"<render fragment="body-0"></render>"#);
    for index in 0..declarations {
        write!(
            source,
            "<fragment name=\"body-{index}\"><span>body-{index}</span>"
        )
        .unwrap_or_else(|error| panic!("writing declaration failed: {error}"));
        if index + 1 < declarations {
            write!(source, "<render fragment=\"body-{}\"></render>", index + 1)
                .unwrap_or_else(|error| panic!("writing call failed: {error}"));
        }
        source.push_str("</fragment>");
    }
    source
}

fn compile_ssr(body: &str) -> WebUIProtocol {
    let mut source = String::with_capacity(body.len() + 13);
    source.push_str("<body>");
    source.push_str(body);
    source.push_str("</body>");
    let mut parser = HtmlParser::new();
    parser
        .parse("index.html", &source)
        .unwrap_or_else(|error| panic!("compiling fragment benchmark failed: {error}"));
    WebUIProtocol::new(parser.into_fragment_records())
}

fn compile_client(source: &str) -> String {
    generate_compiled_template("fragment-bench", source)
        .unwrap_or_else(|error| panic!("compiling client fragment benchmark failed: {error}"))
}

fn graph_counts(protocol: &WebUIProtocol) -> (usize, usize, usize) {
    let mut fragments = 0;
    let mut body_markers = 0;
    let mut raw_bytes = 0;
    for record in protocol.fragments.values() {
        fragments += record.fragments.len();
        for fragment in &record.fragments {
            if let Some(Fragment::Raw(raw)) = &fragment.fragment {
                body_markers += raw.value.matches(BODY_MARKER).count();
                raw_bytes += raw.value.len();
            }
        }
    }
    (fragments, body_markers, raw_bytes)
}

fn report_graph(label: &str, source: &str) -> (usize, usize, usize) {
    let protocol = compile_ssr(source);
    let counts = graph_counts(&protocol);
    let bytes = protocol
        .to_protobuf()
        .unwrap_or_else(|error| panic!("encoding fragment benchmark failed: {error}"));
    let metadata = compile_client(source);
    println!(
        "fragment_compile {label}: source_bytes={}, records={}, fragments={}, raw_bytes={}, \
         protobuf_bytes={}, client_bytes={}, client_blocks={}",
        source.len(),
        protocol.fragments.len(),
        counts.0,
        counts.2,
        bytes.len(),
        metadata.len(),
        metadata.matches("\"h\":").count(),
    );
    (protocol.fragments.len(), counts.0, counts.2)
}

fn bench_repeated_calls(c: &mut Criterion) {
    let mut group = c.benchmark_group("fragment_compile_repeated_calls");
    let mut baseline = None;
    for calls in [1usize, 16, 256, 1024] {
        let source = repeated_calls(calls);
        let protocol = compile_ssr(&source);
        let counts = graph_counts(&protocol);
        assert_eq!(counts.1, 1, "SSR must store the declaration body once");
        let metadata = compile_client(&source);
        assert_eq!(
            metadata.matches(BODY_MARKER).count(),
            1,
            "client metadata must store the declaration body once"
        );
        assert_eq!(
            metadata.matches("\"h\":").count(),
            2,
            "all callsites must share one flat body block"
        );
        let shape = report_graph(&calls.to_string(), &source);
        if let Some((records, fragments, raw_bytes)) = baseline {
            assert_eq!(shape.0, records, "calls must not duplicate body records");
            assert_eq!(shape.1, fragments + calls - 1);
            assert_eq!(
                shape.2, raw_bytes,
                "calls must not duplicate raw body bytes"
            );
        } else {
            baseline = Some(shape);
        }
        group.throughput(Throughput::Bytes(source.len() as u64));
        group.bench_with_input(BenchmarkId::new("ssr", calls), &source, |b, source| {
            b.iter(|| black_box(compile_ssr(black_box(source))));
        });
        group.bench_with_input(BenchmarkId::new("client", calls), &source, |b, source| {
            b.iter(|| black_box(compile_client(black_box(source))));
        });
    }
    group.finish();
}

fn bench_declaration_graph(c: &mut Criterion) {
    let mut group = c.benchmark_group("fragment_compile_declaration_graph");
    for declarations in [1usize, 32, 256, 1024] {
        let source = declaration_chain(declarations);
        report_graph(&format!("chain-{declarations}"), &source);
        let metadata = compile_client(&source);
        assert_eq!(metadata.matches("\"h\":").count(), declarations + 1);
        group.throughput(Throughput::Elements(declarations as u64));
        group.bench_with_input(
            BenchmarkId::new("ssr", declarations),
            &source,
            |b, source| b.iter(|| black_box(compile_ssr(black_box(source)))),
        );
        group.bench_with_input(
            BenchmarkId::new("client", declarations),
            &source,
            |b, source| b.iter(|| black_box(compile_client(black_box(source)))),
        );
    }
    group.finish();
}

pub(super) fn bench_fragments(c: &mut Criterion) {
    bench_repeated_calls(c);
    bench_declaration_graph(c);
}
