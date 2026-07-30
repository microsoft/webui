// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Progressive streaming hydration benchmark.
//!
//! Compares legacy one-shot [`WebUIHandler::render`] with
//! [`WebUIHandler::render_streaming`] for equivalent SSR page markup. The
//! streaming cases use explicit, in-order boundary signals; setup validates
//! their envelope count and boundary-driven flush count before Criterion times
//! the render loop.
//!
//! Browser CPU and heap are intentionally out of scope here. The Playwright
//! harness in `examples/integration/streaming-browser-bench` owns browser-side
//! validation.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use serde_json::{json, Value};
use std::hint::black_box;
use webui_handler::plugin::webui::WebUIHydrationPlugin;
use webui_handler::{
    BoundaryMode, FlushWriter, Protocol, RenderOptions, ResponseWriter, WebUIHandler,
};
use webui_parser::plugin::webui::WebUIParserPlugin;
use webui_parser::{ComponentRegistration, CssStrategy, HtmlParser};
use webui_protocol::StreamingBoundaryList;
use webui_protocol::{ComponentData, InitialStateStrategy, StateProjectionMode, WebUIProtocol};

const BOUNDARY_COUNTS: &[usize] = &[1, 3, 10, 100];
const WRITER_CAPACITY: usize = 32 * 1024;
const ENTRY_ID: &str = "index.html";
const REQUEST_PATH: &str = "/";
const ISLAND_TAG: &str = "bench-island";

struct BenchWriter {
    output: String,
    flushes: Vec<usize>,
}

impl BenchWriter {
    fn new(flush_capacity: usize) -> Self {
        Self {
            output: String::with_capacity(WRITER_CAPACITY),
            flushes: Vec::with_capacity(flush_capacity),
        }
    }

    fn reset(&mut self) {
        self.output.clear();
        self.flushes.clear();
    }
}

impl ResponseWriter for BenchWriter {
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        self.output.push_str(content);
        Ok(())
    }

    fn end(&mut self) -> webui_handler::Result<()> {
        Ok(())
    }
}

impl FlushWriter for BenchWriter {
    fn flush(&mut self) -> webui_handler::Result<()> {
        self.flushes.push(self.output.len());
        Ok(())
    }
}

struct StreamingCase {
    boundaries: usize,
    protocol: Protocol,
    legacy_bytes: usize,
    streaming_bytes: usize,
    flushes: usize,
}

fn hydration_handler() -> WebUIHandler {
    WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()))
}

fn benchmark_state() -> Value {
    json!({
        "count": 42,
        "title": "Hydration benchmark",
        "serverOnly": "not sent to hydration",
    })
}

/// Author the entry-page HTML the benchmark parses. Each boundary is a real
/// `<boundary>` directive wrapping an authored `<article>` and a
/// registered `<bench-island>` component, inside a real `<head>`/`<body>`
/// document so the parser emits the streaming head/body/boundary signals.
fn entry_html(boundaries: usize) -> String {
    let mut html = String::with_capacity(160 + boundaries * 96);
    html.push_str(
        "<!doctype html><html><head>\
         <script type=\"module\" async src=\"/index.js\"></script>\
         </head><body>",
    );
    for sequence in 0..boundaries {
        html.push_str(&format!(
            "<boundary name=\"boundary-{sequence}\">\
             <article data-boundary=\"{sequence}\"><bench-island></bench-island></article>\
             </boundary>"
        ));
    }
    html.push_str("</body></html>");
    html
}

/// Build the timed protocol by running the real `HtmlParser` over authored
/// HTML. The entry fragment graph — head fragmentation, body brackets, and
/// per-boundary `boundary_start`/`boundary_end` signals — is parser-produced;
/// only the island's known `ComponentData` (template + hydration surface) is
/// attached afterward so the benchmark's hydration payload stays deterministic.
fn parser_protocol(boundaries: usize) -> Protocol {
    let entry_html = entry_html(boundaries);
    let mut parser =
        HtmlParser::with_plugin_options(Box::new(WebUIParserPlugin::new()), CssStrategy::Style);
    if let Err(error) = parser
        .component_registry_mut()
        .register_component(ComponentRegistration {
            tag_name: ISLAND_TAG,
            html_content: "<button>{{title}}</button>",
            css_content: None,
            is_client_owned: false,
        })
    {
        panic!("registering <bench-island> failed: {error}");
    }
    if let Err(error) = parser.parse(ENTRY_ID, &entry_html) {
        panic!("parsing benchmark entry failed: {error}");
    }

    let mut document = WebUIProtocol::new(parser.into_fragment_records());
    document.initial_state_strategy = InitialStateStrategy::Components as i32;
    if boundaries > 0 {
        document.streaming_boundaries.insert(
            ENTRY_ID.to_string(),
            StreamingBoundaryList {
                names: (0..boundaries)
                    .map(|index| format!("boundary-{index}"))
                    .collect(),
            },
        );
    }
    // Attach the benchmark's known component surface after parsing so the timed
    // protocol carries a deterministic hydration payload without depending on
    // the plugin's artifact pipeline.
    document.components.insert(
        ISLAND_TAG.to_string(),
        ComponentData {
            template_json: r#"{"h":"<button></button>","th":1}"#.to_string(),
            hydration_mode: StateProjectionMode::Keys as i32,
            hydration_keys: vec!["count".to_string(), "title".to_string()],
            ..Default::default()
        },
    );
    Protocol::new(document)
}

fn occurrences(haystack: &str, needle: &str) -> usize {
    haystack.match_indices(needle).count()
}

fn render_legacy(
    handler: &WebUIHandler,
    protocol: &Protocol,
    state: &Value,
    writer: &mut BenchWriter,
) {
    if let Err(error) = handler.render(
        protocol,
        state,
        &RenderOptions::new(ENTRY_ID, REQUEST_PATH),
        writer,
    ) {
        panic!("legacy render failed: {error}");
    }
}

fn render_streaming(
    handler: &WebUIHandler,
    protocol: &Protocol,
    state: &Value,
    writer: &mut BenchWriter,
) {
    if let Err(error) = handler.render_streaming(
        protocol,
        state,
        &RenderOptions::new(ENTRY_ID, REQUEST_PATH),
        writer,
    ) {
        panic!("streaming render failed: {error}");
    }
}

fn verify_streaming_case(boundaries: usize) -> StreamingCase {
    let protocol = parser_protocol(boundaries);
    let handler = hydration_handler();
    let state = benchmark_state();
    let mut legacy = BenchWriter::new(0);
    let mut streaming = BenchWriter::new(boundaries + 1);

    render_legacy(&handler, &protocol, &state, &mut legacy);
    render_streaming(&handler, &protocol, &state, &mut streaming);

    assert!(
        legacy.flushes.is_empty(),
        "legacy rendering must not request streaming flushes"
    );
    assert_eq!(
        streaming.flushes.len(),
        boundaries + 1,
        "each explicit boundary and the terminal record must flush once"
    );
    assert_eq!(
        occurrences(&streaming.output, "data-webui-boundary"),
        boundaries + 1,
        "each explicit boundary plus the terminal record needs one envelope"
    );
    assert_eq!(
        occurrences(&streaming.output, "<webui-hydrate>"),
        boundaries + 1,
        "each envelope needs one hydration sentinel"
    );
    assert!(
        !streaming.output.contains("id=\"webui-data\""),
        "streaming output must not include the legacy page-wide bootstrap"
    );
    assert!(
        !legacy.output.contains("data-webui-boundary"),
        "legacy output must not include streaming envelopes"
    );
    // The parser emits a `streaming_root` signal inside each in-boundary
    // component host; streaming render consumes it to inject exactly ` data-ws`
    // before custom-element upgrade, while legacy render ignores it.
    assert!(
        streaming.output.contains("<bench-island data-ws>"),
        "streamed island host must carry data-ws"
    );
    assert!(
        !legacy.output.contains("data-ws"),
        "legacy render must not inject the streaming host marker"
    );
    // Envelope-local template dedup: the island template is sent once (in the
    // first boundary that renders it) and reused by every later boundary via
    // hydration state only — never re-sent as a duplicate template.
    assert_eq!(
        occurrences(&streaming.output, "\"bench-island\":{"),
        1,
        "the island template must be emitted once and reused, not per boundary"
    );

    for sequence in 0..boundaries {
        // The `<article>` wrapper is authored HTML the parser passes through as
        // a raw fragment, so it anchors the boundary's page markup in both
        // render modes without coupling to the island's internal rendering.
        let page_markup = format!("<article data-boundary=\"{sequence}\">");
        assert!(
            legacy.output.contains(&page_markup),
            "legacy output is missing boundary {sequence} page markup"
        );
        assert!(
            streaming.output.contains(&page_markup),
            "streaming output is missing boundary {sequence} page markup"
        );
        assert!(
            streaming.output.contains(&format!("<!--wb:{sequence}-->")),
            "streaming output is missing boundary {sequence} start marker"
        );
        assert!(
            streaming.output.contains(&format!("<!--/wb:{sequence}-->")),
            "streaming output is missing boundary {sequence} end marker"
        );
        assert!(
            streaming
                .output
                .contains(&format!("[1,{sequence},0,{sequence},")),
            "streaming output is missing boundary {sequence} envelope"
        );
    }

    assert!(
        streaming.flushes.windows(2).all(|pair| pair[0] < pair[1]),
        "flush positions must advance in document order"
    );
    assert!(
        streaming
            .output
            .contains(&format!("[1,{boundaries},3,0,{{}}]")),
        "streaming output is missing the terminal envelope"
    );
    // The empty terminal record is always the last envelope and never carries a
    // bootstrap, regardless of native or scriptless tail bytes.
    // Every envelope (each boundary commit plus the terminal) opens with the
    // boundary sentinel prefix, so the prefix count equals boundaries + 1.
    assert_eq!(
        occurrences(&streaming.output, "data-webui-boundary>[1,"),
        boundaries + 1,
        "each envelope opens with the boundary sentinel prefix"
    );
    if let Some(terminal) = streaming.output.find(&format!("[1,{boundaries},3,0,{{}}]")) {
        if boundaries > 0 {
            match streaming
                .output
                .rfind(&format!("<!--/wb:{}-->", boundaries - 1))
            {
                Some(last_boundary) => assert!(
                    last_boundary < terminal,
                    "terminal record must trail the final boundary commit"
                ),
                None => panic!("streaming output is missing the final boundary end marker"),
            }
        }
    }

    println!(
        "streaming_hydration boundaries={boundaries}: legacy_bytes={}, streaming_bytes={}, flushes={}",
        legacy.output.len(),
        streaming.output.len(),
        streaming.flushes.len()
    );

    StreamingCase {
        boundaries,
        protocol,
        legacy_bytes: legacy.output.len(),
        streaming_bytes: streaming.output.len(),
        flushes: streaming.flushes.len(),
    }
}

fn verify_ordinary_render() -> usize {
    let handler = hydration_handler();
    let state = benchmark_state();
    // A parser protocol that carries head_start and several boundary signals:
    // ordinary render() must treat every streaming signal as inert, emitting the
    // page-wide bootstrap and none of the streaming wire artifacts. This guards
    // against a streaming-path change leaking into ordinary render().
    let boundaried_protocol = parser_protocol(3);
    let ordinary_protocol = parser_protocol(0);
    let mut boundaried_writer = BenchWriter::new(0);
    let mut ordinary_writer = BenchWriter::new(0);

    render_legacy(
        &handler,
        &boundaried_protocol,
        &state,
        &mut boundaried_writer,
    );
    render_legacy(&handler, &ordinary_protocol, &state, &mut ordinary_writer);

    assert!(
        boundaried_writer.flushes.is_empty() && ordinary_writer.flushes.is_empty(),
        "ordinary render() must not flush"
    );
    assert!(
        boundaried_writer.output.contains("id=\"webui-data\""),
        "ordinary render() must emit the page-wide #webui-data bootstrap"
    );
    for artifact in [
        "data-webui-boundary",
        "<webui-hydrate>",
        "<!--wb:",
        "webui-streaming",
        "data-ws",
    ] {
        assert!(
            !boundaried_writer.output.contains(artifact),
            "ordinary render() must not emit streaming artifact {artifact:?}"
        );
    }

    println!(
        "streaming_hydration ordinary_render boundaries=0: output_bytes={}, flushes=0",
        ordinary_writer.output.len()
    );
    ordinary_writer.output.len()
}

fn render_state_updates(
    handler: &WebUIHandler,
    protocol: &Protocol,
    state: &Value,
    updates: usize,
    writer: &mut BenchWriter,
) {
    let options = RenderOptions::new(ENTRY_ID, REQUEST_PATH);
    let mut response = match handler.stream_response(protocol, &options, writer) {
        Ok(response) => response,
        Err(error) => panic!("starting streaming response failed: {error}"),
    };
    let boundary = match response.boundary("boundary-0") {
        Ok(boundary) => boundary,
        Err(error) => panic!("resolving benchmark boundary failed: {error}"),
    };
    if let Err(error) = response.write_shell(state) {
        panic!("writing benchmark shell failed: {error}");
    }
    if let Err(error) = response.write_boundary(boundary, state, BoundaryMode::Updatable) {
        panic!("writing benchmark boundary failed: {error}");
    }
    for _ in 0..updates {
        if let Err(error) = response.update(boundary, state) {
            panic!("writing benchmark state update failed: {error}");
        }
    }
    if let Err(error) = response.finish(state) {
        panic!("finishing benchmark response failed: {error}");
    }
}

fn verify_state_updates(updates: usize) -> (Protocol, usize, usize) {
    let protocol = parser_protocol(1);
    let handler = hydration_handler();
    let state = benchmark_state();
    let mut writer = BenchWriter::new(updates + 3);
    render_state_updates(&handler, &protocol, &state, updates, &mut writer);

    assert_eq!(
        writer.flushes.len(),
        updates + 3,
        "shell, checkpoint, updates, and terminal must flush independently",
    );
    assert_eq!(
        occurrences(&writer.output, ",2,0,"),
        updates,
        "each update needs one typed state-update record",
    );
    assert_eq!(
        occurrences(&writer.output, "\"serverOnly\""),
        0,
        "updates must reuse the compiled boundary projection",
    );
    println!(
        "streaming_state_updates updates={updates}: output_bytes={}, flushes={}",
        writer.output.len(),
        writer.flushes.len(),
    );
    (protocol, writer.output.len(), writer.flushes.len())
}

fn bench_streaming_hydration(c: &mut Criterion) {
    let state = benchmark_state();
    let cases: Vec<StreamingCase> = BOUNDARY_COUNTS
        .iter()
        .copied()
        .map(verify_streaming_case)
        .collect();
    let ordinary_bytes = verify_ordinary_render();
    let mut ordinary_group = c.benchmark_group("streaming_hydration_ordinary_render");
    ordinary_group.throughput(Throughput::Bytes(ordinary_bytes as u64));
    ordinary_group.bench_function("no_boundaries", |b| {
        let handler = hydration_handler();
        // Time the real parser-produced page shape (a streaming-capable document
        // still emits head_start, which ordinary render() treats as a no-op —
        // asserted in `verify_ordinary_render`).
        let protocol = parser_protocol(0);
        let mut writer = BenchWriter::new(0);

        b.iter(|| {
            writer.reset();
            render_legacy(
                &handler,
                black_box(&protocol),
                black_box(&state),
                &mut writer,
            );
            black_box(writer.output.len());
        });
    });
    ordinary_group.finish();

    let mut group = c.benchmark_group("streaming_hydration_boundaries");
    for case in &cases {
        group.throughput(Throughput::Bytes(case.legacy_bytes as u64));
        group.bench_with_input(
            BenchmarkId::new("legacy_one_shot", case.boundaries),
            case,
            |b, case| {
                let handler = hydration_handler();
                let mut writer = BenchWriter::new(0);

                b.iter(|| {
                    writer.reset();
                    render_legacy(
                        &handler,
                        black_box(&case.protocol),
                        black_box(&state),
                        &mut writer,
                    );
                    black_box(writer.output.len());
                });
            },
        );

        group.throughput(Throughput::Bytes(case.streaming_bytes as u64));
        group.bench_with_input(
            BenchmarkId::new("streaming_hydration", case.boundaries),
            case,
            |b, case| {
                let handler = hydration_handler();
                let mut writer = BenchWriter::new(case.flushes);

                b.iter(|| {
                    writer.reset();
                    render_streaming(
                        &handler,
                        black_box(&case.protocol),
                        black_box(&state),
                        &mut writer,
                    );
                    black_box((writer.output.len(), writer.flushes.len()));
                });
            },
        );
    }
    group.finish();

    let mut update_group = c.benchmark_group("streaming_state_updates");
    for updates in [1usize, 10, 100] {
        let (protocol, output_bytes, flushes) = verify_state_updates(updates);
        update_group.throughput(Throughput::Bytes(output_bytes as u64));
        update_group.bench_with_input(
            BenchmarkId::from_parameter(updates),
            &updates,
            |b, &updates| {
                let handler = hydration_handler();
                let mut writer = BenchWriter::new(flushes);
                b.iter(|| {
                    writer.reset();
                    render_state_updates(
                        &handler,
                        black_box(&protocol),
                        black_box(&state),
                        updates,
                        &mut writer,
                    );
                    black_box((writer.output.len(), writer.flushes.len()));
                });
            },
        );
    }
    update_group.finish();
}

criterion_group!(benches, bench_streaming_hydration);
criterion_main!(benches);
