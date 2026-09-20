// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

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

use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};
use serde_json::{json, Value};
use std::hint::black_box;
use std::sync::Arc;
use webui_handler::plugin::webui::WebUIHydrationPlugin;
use webui_handler::{
    BoundaryMode, FlushWriter, Protocol, RenderOptions, ResponseWriter, SessionOptions,
    StreamingSession, WebUIHandler,
};
use webui_parser::plugin::webui::WebUIParserPlugin;
use webui_parser::{ComponentRegistration, CssStrategy, HtmlParser};
use webui_protocol::{ComponentData, InitialStateStrategy, StateProjectionMode, WebUIProtocol};

const BOUNDARY_COUNTS: &[usize] = &[1, 3, 10, 100];
const LARGE_STATE_BOUNDARIES: &[usize] = &[1, 8];
const LARGE_STATE_ROWS: usize = 128;
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

    webui_handler::string_response_writer_methods!(output);

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
/// only the island's known runtime metadata and style closures are attached
/// afterward so the benchmark's hydration payload stays deterministic.
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
    // Attach the benchmark's known component surface after parsing so the timed
    // protocol carries a deterministic hydration payload without depending on
    // the plugin's artifact pipeline.
    document.components.insert(
        ISLAND_TAG.to_string(),
        ComponentData {
            template_json: r#"{"h":"<button></button>","th":1}"#.to_string(),
            uses_shadow_dom: true,
            hydration_mode: StateProjectionMode::Keys as i32,
            hydration_keys: vec!["count".to_string(), "title".to_string()],
            ..Default::default()
        },
    );
    document.populate_style_closures(&[ENTRY_ID]);
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

fn render_split(
    handler: &WebUIHandler,
    protocol: &Protocol,
    state: &Value,
    writer: &mut BenchWriter,
) {
    let options = RenderOptions::new(ENTRY_ID, REQUEST_PATH);
    let mut response = match handler.stream_response(protocol, &options, writer) {
        Ok(response) => response,
        Err(error) => panic!("starting split streaming response failed: {error}"),
    };
    let mut status = match response.start(state) {
        Ok(status) => status,
        Err(error) => panic!("starting split streaming traversal failed: {error}"),
    };
    while !status.done {
        status = match status.boundary.as_ref() {
            Some(boundary) => {
                match response.resume_current(boundary.instance_id, BoundaryMode::Final) {
                    Ok(status) => status,
                    Err(error) => panic!("resuming split streaming traversal failed: {error}"),
                }
            }
            None => match response.advance() {
                Ok(status) => status,
                Err(error) => panic!("advancing split streaming traversal failed: {error}"),
            },
        };
    }
}

fn render_owned_split(
    handler: Arc<WebUIHandler>,
    protocol: Arc<Protocol>,
    states: Vec<Value>,
) -> usize {
    let mut session = match StreamingSession::new(
        handler,
        protocol,
        SessionOptions::new(ENTRY_ID, REQUEST_PATH),
    ) {
        Ok(session) => session,
        Err(error) => panic!("creating owned streaming session failed: {error}"),
    };
    let mut states = states.into_iter();
    let initial_state = match states.next() {
        Some(state) => state,
        None => panic!("owned streaming benchmark requires initial state"),
    };
    let mut step = match session.start(initial_state) {
        Ok(step) => step,
        Err(error) => panic!("starting owned streaming traversal failed: {error}"),
    };
    let mut bytes = step.bytes.len();
    while !step.done {
        step = match step.boundary.as_ref() {
            Some(boundary) => match states.next() {
                Some(state) => {
                    match session.resume(boundary.instance_id, state, BoundaryMode::Final) {
                        Ok(step) => step,
                        Err(error) => {
                            panic!("resuming owned streaming traversal failed: {error}")
                        }
                    }
                }
                None => {
                    panic!("owned streaming benchmark requires state for every boundary")
                }
            },
            None => match session.advance() {
                Ok(step) => step,
                Err(error) => panic!("advancing owned streaming traversal failed: {error}"),
            },
        };
        bytes += step.bytes.len();
    }
    bytes
}

fn verify_streaming_case(boundaries: usize) -> StreamingCase {
    let protocol = parser_protocol(boundaries);
    let handler = hydration_handler();
    let state = benchmark_state();
    let mut legacy = BenchWriter::new(0);
    let mut streaming = BenchWriter::new(boundaries + 1);

    render_legacy(&handler, &protocol, &state, &mut legacy);
    render_streaming(&handler, &protocol, &state, &mut streaming);

    assert_eq!(
        occurrences(&legacy.output, "<template shadowrootmode=\"open\">"),
        boundaries,
        "legacy rendering must honor the parser-produced Shadow roots"
    );
    assert_eq!(
        occurrences(&streaming.output, "<template shadowrootmode=\"open\">"),
        boundaries,
        "streaming rendering must honor the parser-produced Shadow roots"
    );
    assert!(
        legacy.flushes.is_empty(),
        "legacy rendering must not request streaming flushes"
    );
    // Every boundary commits a checkpoint that flushes, plus the terminal
    // record. A semantic step that returns without producing bytes since the
    // checkpoint (adjacent boundaries) collapses into the checkpoint flush, so
    // only steps that actually emit a prefix add one.
    assert_eq!(
        streaming.flushes.len(),
        boundaries + 2,
        "each checkpoint flushes, plus the shell prefix and the terminal"
    );
    assert!(
        streaming.flushes.windows(2).all(|pair| pair[0] < pair[1]),
        "every flush must release bytes that the previous flush did not"
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
                .contains(&format!("[{sequence},0,{sequence},")),
            "streaming output is missing boundary {sequence} envelope"
        );
    }

    assert!(
        streaming
            .output
            .contains(&format!("[{boundaries},4,0,{{}}]")),
        "streaming output is missing the terminal envelope"
    ); // The empty terminal record is always the last envelope and never carries a
       // bootstrap, regardless of native or scriptless tail bytes.
       // Every envelope (each boundary commit plus the terminal) opens with the
       // boundary sentinel prefix, so the prefix count equals boundaries + 1.
    assert_eq!(
        occurrences(&streaming.output, "data-webui-boundary>["),
        boundaries + 1,
        "each envelope opens with the boundary sentinel prefix"
    );
    if let Some(terminal) = streaming.output.find(&format!("[{boundaries},4,0,{{}}]")) {
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
    let first = match response.start(state) {
        Ok(status) => status.boundary,
        Err(error) => panic!("writing benchmark shell failed: {error}"),
    };
    let Some(first) = first else {
        panic!("benchmark response did not discover its first boundary");
    };
    if let Err(error) = response.resume(first.instance_id, state, BoundaryMode::Updatable) {
        panic!("writing benchmark boundary failed: {error}");
    }
    // Updates land between the commit and the parent bytes that follow it, so
    // the benchmark exercises the live-occurrence window a host actually uses.
    for _ in 0..updates {
        if let Err(error) = response.update(first.instance_id, state) {
            panic!("writing benchmark state update failed: {error}");
        }
    }
    let second = match response.advance() {
        Ok(status) => status.boundary,
        Err(error) => panic!("advancing past the benchmark boundary failed: {error}"),
    };
    let Some(second) = second else {
        panic!("benchmark response completed before its second boundary");
    };
    if let Err(error) = response.resume(second.instance_id, state, BoundaryMode::Final) {
        panic!("writing the second benchmark boundary failed: {error}");
    }
    match response.advance() {
        Ok(status) if status.done => {}
        Ok(_) => panic!("benchmark response did not complete"),
        Err(error) => panic!("finishing benchmark response failed: {error}"),
    }
}

fn verify_state_updates(updates: usize) -> (Protocol, usize, usize) {
    let protocol = parser_protocol(2);
    let handler = hydration_handler();
    let state = benchmark_state();
    let mut writer = BenchWriter::new(updates + 4);
    render_state_updates(&handler, &protocol, &state, updates, &mut writer);

    assert_eq!(
        writer.flushes.len(),
        updates + 4,
        "shell prefix, two checkpoints, every update, and the terminal flush independently",
    );
    assert_eq!(
        occurrences(&writer.output, ",2,0,{"),
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

    bench_large_state_boundaries(c);
}

/// Time a full-state continuation across several boundaries.
///
/// A protocol whose reachable component projects `ALL` forces the response to
/// retain the caller's whole state for the life of the response. The per-
/// boundary cost must stay flat in the size of that state: the snapshot is
/// taken once when the response starts, not re-merged at every occurrence.
fn bench_large_state_boundaries(c: &mut Criterion) {
    let state = large_state(LARGE_STATE_ROWS);
    let mut group = c.benchmark_group("streaming_large_state");
    for boundaries in LARGE_STATE_BOUNDARIES.iter().copied() {
        let protocol = full_state_protocol(boundaries);
        let handler = hydration_handler();
        let mut legacy = BenchWriter::new(0);
        render_legacy(&handler, &protocol, &state, &mut legacy);
        let mut writer = BenchWriter::new(boundaries + 2);
        render_streaming(&handler, &protocol, &state, &mut writer);
        let bytes = writer.output.len();
        assert_eq!(
            occurrences(&writer.output, "data-webui-boundary"),
            boundaries + 1,
            "each boundary plus the terminal needs one envelope"
        );
        assert_eq!(
            occurrences(&writer.output, "\"rows\":["),
            1,
            "unchanged full state must be serialized once per response"
        );
        println!(
            "streaming_large_state boundaries={boundaries}: rows={LARGE_STATE_ROWS}, buffered_bytes={}, streaming_bytes={bytes}",
            legacy.output.len(),
        );

        group.throughput(Throughput::Bytes(bytes as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(boundaries),
            &protocol,
            |b, protocol| {
                let handler = hydration_handler();
                let mut writer = BenchWriter::new(boundaries + 2);
                b.iter(|| {
                    writer.reset();
                    render_streaming(
                        &handler,
                        black_box(protocol),
                        black_box(&state),
                        &mut writer,
                    );
                    black_box(writer.output.len());
                });
            },
        );
    }
    group.finish();

    let boundaries = 8;
    let protocol = full_state_protocol(boundaries);
    let handler = hydration_handler();
    let mut legacy = BenchWriter::new(0);
    render_legacy(&handler, &protocol, &state, &mut legacy);
    let mut streaming = BenchWriter::new(boundaries + 2);
    render_streaming(&handler, &protocol, &state, &mut streaming);
    let expected_streaming_bytes = streaming.output.len();
    let mut compare = c.benchmark_group("streaming_large_state_compare");
    compare.throughput(Throughput::Bytes(legacy.output.len() as u64));
    compare.bench_function("buffered", |b| {
        let handler = hydration_handler();
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
    compare.throughput(Throughput::Bytes(expected_streaming_bytes as u64));
    compare.bench_function("streaming_fused", |b| {
        let handler = hydration_handler();
        let mut writer = BenchWriter::new(boundaries + 2);
        b.iter(|| {
            writer.reset();
            render_streaming(
                &handler,
                black_box(&protocol),
                black_box(&state),
                &mut writer,
            );
            black_box(writer.output.len());
        });
    });
    compare.bench_function("streaming_split", |b| {
        let handler = hydration_handler();
        let mut writer = BenchWriter::new(boundaries + 2);
        b.iter(|| {
            writer.reset();
            render_split(
                &handler,
                black_box(&protocol),
                black_box(&state),
                &mut writer,
            );
            black_box(writer.output.len());
        });
    });
    let owned_handler = Arc::new(hydration_handler());
    let owned_protocol = Arc::new(full_state_protocol(boundaries));
    compare.bench_function("streaming_owned_split", |b| {
        b.iter_batched(
            || {
                let mut states = Vec::with_capacity(boundaries + 1);
                states.push(state.clone());
                states.resize_with(boundaries + 1, || Value::Object(serde_json::Map::new()));
                states
            },
            |owned_states| {
                black_box(render_owned_split(
                    Arc::clone(&owned_handler),
                    Arc::clone(&owned_protocol),
                    black_box(owned_states),
                ));
            },
            BatchSize::SmallInput,
        );
    });
    compare.finish();
}

/// Build the same authored page as [`parser_protocol`] with an island whose
/// compiled hydration surface is `ALL`, which is what forces the continuation
/// to retain full state.
fn full_state_protocol(boundaries: usize) -> Protocol {
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
    document.components.insert(
        ISLAND_TAG.to_string(),
        ComponentData {
            template_json: r#"{"h":"<button></button>","th":1}"#.to_string(),
            uses_shadow_dom: true,
            hydration_mode: StateProjectionMode::All as i32,
            ..Default::default()
        },
    );
    document.populate_style_closures(&[ENTRY_ID]);
    Protocol::new(document)
}

/// A state whose payload makes a per-boundary copy unmistakable.
fn large_state(rows: usize) -> Value {
    let mut items = Vec::with_capacity(rows);
    for row in 0..rows {
        items.push(json!({
            "id": row,
            "label": format!("row-{row}"),
            "tags": ["alpha", "beta", "gamma"],
        }));
    }
    json!({
        "count": 42,
        "title": "Hydration benchmark",
        "rows": items,
    })
}

criterion_group!(benches, bench_streaming_hydration);
criterion_main!(benches);
