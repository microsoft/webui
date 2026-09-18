// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput};
use serde_json::{Map, Value};
use std::hint::black_box;
use webui_handler::{BoundaryMode, Protocol, RenderOptions, WebUIHandler};
use webui_protocol::InitialStateStrategy;

use crate::fragment_support::{compile_document, EvidenceWriter, ENTRY};

fn source(calls: usize) -> String {
    let mut source = String::with_capacity(calls * 72 + 400);
    source.push_str("<!doctype html><html><head></head><body>");
    for _ in 0..calls {
        source.push_str(r#"<render fragment="selected" scope="{{selected}}" as="value"></render>"#);
    }
    source.push_str(
        r#"<footer>{{selected.label}}</footer><fragment name="selected"><p class="before">{{value.label}}</p><boundary name="selected-ready" key="{{value.id}}"><b>{{value.label}}</b></boundary><p class="after">{{value.label}}</p></fragment></body></html>"#,
    );
    source
}

fn owned_states(calls: usize, payload_bytes: usize) -> Vec<Value> {
    let mut states = Vec::with_capacity(calls + 1);
    for index in 0..=calls {
        let selected = Value::Object(Map::from_iter([
            ("id".to_string(), Value::String(index.to_string())),
            (
                "label".to_string(),
                Value::String(format!("selected-{index}")),
            ),
            (
                "unused".to_string(),
                Value::String("x".repeat(payload_bytes)),
            ),
        ]));
        states.push(Value::Object(Map::from_iter([(
            "selected".to_string(),
            selected,
        )])));
    }
    states
}

fn render_replaced(
    handler: &WebUIHandler,
    protocol: &Protocol,
    states: Vec<Value>,
    writer: &mut EvidenceWriter,
) -> usize {
    let options = RenderOptions::new(ENTRY, "/");
    let mut response = handler
        .stream_response(protocol, &options, writer)
        .unwrap_or_else(|error| panic!("creating fragment response failed: {error}"));
    let mut states = states.into_iter();
    let initial = states
        .next()
        .unwrap_or_else(|| panic!("fragment response needs initial state"));
    let mut status = response
        .start(initial)
        .unwrap_or_else(|error| panic!("starting fragment response failed: {error}"));
    let mut boundaries = 0;
    while !status.done {
        status = if let Some(boundary) = status.boundary.as_ref() {
            boundaries += 1;
            let replacement = states
                .next()
                .unwrap_or_else(|| panic!("fragment response needs a replacement root"));
            response
                .resume(boundary.instance_id, replacement, BoundaryMode::Final)
                .unwrap_or_else(|error| panic!("resuming fragment response failed: {error}"))
        } else {
            response
                .advance()
                .unwrap_or_else(|error| panic!("advancing fragment response failed: {error}"))
        };
    }
    assert!(
        states.next().is_none(),
        "every replacement root must be used"
    );
    boundaries
}

fn verify_inputs(writer: &EvidenceWriter, calls: usize) {
    for index in 0..calls {
        for (opening, closing) in [
            ("<p class=\"before\">", "</p>"),
            ("<b>", "</b>"),
            ("<p class=\"after\">", "</p>"),
        ] {
            let expected = format!("{opening}selected-{index}{closing}");
            assert_eq!(
                writer.output.matches(&expected).count(),
                1,
                "the invocation must retain its input before, during and after suspension"
            );
        }
    }
    assert!(
        writer
            .output
            .contains(&format!("<footer>selected-{calls}</footer>")),
        "owner lookup after all calls must see the latest replaced root"
    );
}

pub(super) fn bench_replaced_roots(c: &mut Criterion) {
    let mut group = c.benchmark_group("fragment_streaming_replaced_roots");
    for calls in [1usize, 8, 32] {
        let mut document = compile_document(&source(calls));
        document.initial_state_strategy = InitialStateStrategy::Components as i32;
        document.populate_style_closures(&[ENTRY]);
        let protocol = Protocol::new(document);
        let handler = WebUIHandler::new();
        let mut expected_output = None;
        for payload_bytes in [0usize, 64 * 1024] {
            let mut warmup = EvidenceWriter::new(calls * 1024 + 1024);
            assert_eq!(
                render_replaced(
                    &handler,
                    &protocol,
                    owned_states(calls, payload_bytes),
                    &mut warmup,
                ),
                calls,
            );
            verify_inputs(&warmup, calls);
            if let Some(expected) = &expected_output {
                assert_eq!(
                    &warmup.output, expected,
                    "unused payload must not reach HTML"
                );
            } else {
                expected_output = Some(warmup.output.clone());
            }
            let mut writer = EvidenceWriter::new(warmup.output.len() + 64);
            render_replaced(
                &handler,
                &protocol,
                owned_states(calls, payload_bytes),
                &mut writer,
            );
            assert_eq!(writer.output, warmup.output);
            let (bytes, writes, growths, flushes) = writer.evidence();
            assert_eq!(growths, 0, "timed writer must not reallocate");
            assert!(flushes >= calls, "each committed boundary must flush");
            println!(
                "fragment_streaming calls={calls}, input_payload_bytes={payload_bytes}: \
                 output_bytes={bytes}, write_calls={writes}, writer_growths={growths}, \
                 flushes={flushes}, retained_inputs_verified={calls}",
            );
            group.throughput(Throughput::Bytes(bytes as u64));
            group.bench_function(
                BenchmarkId::new(format!("calls_{calls}"), payload_bytes),
                |b| {
                    b.iter_batched(
                        || owned_states(calls, payload_bytes),
                        |states| {
                            writer.clear();
                            black_box(render_replaced(
                                &handler,
                                black_box(&protocol),
                                states,
                                &mut writer,
                            ));
                            black_box(writer.evidence());
                        },
                        BatchSize::PerIteration,
                    );
                },
            );
        }
    }
    group.finish();
}
