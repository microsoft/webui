// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Bootstrap state-serialization benchmark.
//!
//! Measures the cost of emitting the `#webui-data` bootstrap block, whose
//! `state` field is streamed on every full-HTML render
//! (`write_webui_bootstrap` → `write_projected_state`).
//!
//! The projected-hydration design filters the SSR state down to only the keys
//! a component actually hydrates (the build-time `hydration_schema`) before
//! streaming it through a single-pass `</`-escaping writer. This benchmark
//! pins that behavior with three arms per size:
//!
//! * `projected_full` — schema contains EVERY top-level key, so the entire
//!   state is serialized. This is the anti-regression guard: the streaming
//!   projection path must not be slower than the monolithic baseline it
//!   replaced (~1.557 ms at 1 MB).
//! * `projected_typical` — schema contains only the small metadata keys, so
//!   the large `items` array is projected away. This is the realistic win: the
//!   emitted block stays tiny and roughly flat regardless of total state size.
//! * `without_plugin` — no bootstrap block is emitted at all; the structural
//!   lower bound.
//!
//! The protocol is intentionally minimal — a bare `<body>` plus a raw
//! `body_end` signal that triggers the bootstrap emission — so the measured
//! work is dominated by state projection + serialization, not template
//! rendering.
//!
//! Sizes span 64 KB / 256 KB / 1 MB of serialized state to expose the linear
//! cost that the projection is designed to collapse for the typical payload.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::hint::black_box;
use webui_handler::plugin::webui::WebUIHydrationPlugin;
use webui_handler::{RenderOptions, ResponseWriter, WebUIHandler};
use webui_protocol::{FragmentList, WebUIFragment, WebUIProtocol};

struct BenchWriter {
    output: String,
}

impl BenchWriter {
    fn new(capacity: usize) -> Self {
        Self {
            output: String::with_capacity(capacity),
        }
    }

    fn clear(&mut self) {
        self.output.clear();
    }

    fn len(&self) -> usize {
        self.output.len()
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

/// Build a realistically-shaped application state whose serialized JSON is at
/// least `target_bytes`.
///
/// The shape mirrors a typical SSR payload: a top-level object with metadata
/// plus a large `items` array of records mixing scalar kinds (int, string,
/// bool, float) and small nested arrays. Record count is estimated up front to
/// avoid repeated serialization; the caller measures the exact serialized size
/// for throughput reporting.
fn build_large_state(target_bytes: usize) -> Value {
    // Empirically ~182 bytes of serialized JSON per record (see bench docs).
    const APPROX_RECORD_BYTES: usize = 182;
    let count = target_bytes / APPROX_RECORD_BYTES + 1;

    let mut items = Vec::with_capacity(count);
    for idx in 0..count {
        items.push(json!({
            "id": idx,
            "name": format!("User {idx:06}"),
            "email": format!("user{idx:06}@example.com"),
            "active": idx % 3 == 0,
            "score": (idx % 100) as f64 + 0.5,
            "roles": ["reader", "writer"],
            "bio": "Lorem ipsum dolor sit amet, consectetur adipiscing elit.",
        }));
    }

    json!({
        "title": "Bootstrap State Benchmark",
        "generatedAt": "2024-01-01T00:00:00Z",
        "count": count,
        "items": items,
    })
}

/// Every top-level key of [`build_large_state`], sorted. Projecting against
/// this emits the full state (worst case / anti-regression guard).
fn full_schema() -> Vec<String> {
    let mut schema = vec![
        "count".to_string(),
        "generatedAt".to_string(),
        "items".to_string(),
        "title".to_string(),
    ];
    schema.sort();
    schema
}

/// The small metadata keys only, sorted — the large `items` array is
/// deliberately excluded so projection collapses the payload (typical case).
fn typical_schema() -> Vec<String> {
    let mut schema = vec![
        "count".to_string(),
        "generatedAt".to_string(),
        "title".to_string(),
    ];
    schema.sort();
    schema
}

/// Minimal full-HTML protocol that fires a single `body_end` hook, carrying the
/// supplied hydration `schema` used to project the state at emission time.
///
/// The raw `body_end` signal resolves to no value (it is a structural hook, not
/// a state key), so it emits no text of its own but triggers the bootstrap
/// `#webui-data` block when a hydration plugin is active.
fn build_bootstrap_protocol(schema: Vec<String>) -> WebUIProtocol {
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("<!DOCTYPE html><html><body>"),
                WebUIFragment::signal("body_end", true),
                WebUIFragment::raw("</body></html>"),
            ],
        },
    );
    let mut protocol = WebUIProtocol::new(fragments);
    protocol.hydration_schema = schema;
    protocol
}

fn serialized_len(state: &Value) -> usize {
    serde_json::to_vec(state)
        .map(|bytes| bytes.len())
        .unwrap_or(0)
}

fn bootstrap_state_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("bootstrap_state");
    let options = RenderOptions::new("index.html", "/");

    // Protocols differ only by the projection schema; state is shared per size.
    let full_protocol = build_bootstrap_protocol(full_schema());
    let typical_protocol = build_bootstrap_protocol(typical_schema());
    let empty_protocol = build_bootstrap_protocol(Vec::new());

    for &target in &[64 * 1024usize, 256 * 1024, 1024 * 1024] {
        let state = build_large_state(target);
        let state_bytes = serialized_len(&state);
        let label = format!("{}KB", target / 1024);

        // Throughput is reported against the actual serialized state size so
        // results read as MB/s of state pushed through the bootstrap block.
        group.throughput(Throughput::Bytes(state_bytes as u64));

        // FULL projection: every top-level key is hydratable, so the entire
        // state is streamed. Anti-regression guard against the monolithic
        // baseline (~1.557 ms at 1 MB).
        group.bench_with_input(
            BenchmarkId::new("projected_full", &label),
            &state,
            |b, st| {
                let handler = WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()));
                let mut writer = BenchWriter::new(state_bytes + 4096);
                b.iter(|| {
                    writer.clear();
                    handler
                        .handle(
                            black_box(&full_protocol),
                            black_box(st),
                            &options,
                            &mut writer,
                        )
                        .unwrap_or_else(|error| panic!("projected_full render failed: {error}"));
                    black_box(writer.len());
                });
            },
        );

        // TYPICAL projection: only the small metadata keys are hydratable, so
        // the large `items` array is projected away and the emitted block stays
        // tiny regardless of total state size. This is the redesign's win.
        group.bench_with_input(
            BenchmarkId::new("projected_typical", &label),
            &state,
            |b, st| {
                let handler = WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()));
                let mut writer = BenchWriter::new(4096);
                b.iter(|| {
                    writer.clear();
                    handler
                        .handle(
                            black_box(&typical_protocol),
                            black_box(st),
                            &options,
                            &mut writer,
                        )
                        .unwrap_or_else(|error| panic!("projected_typical render failed: {error}"));
                    black_box(writer.len());
                });
            },
        );

        // WITHOUT a plugin: no bootstrap block is emitted, so the state is
        // never touched. Structural lower bound. `empty_protocol` carries an
        // empty schema, but with no plugin active no block is emitted, so the
        // schema is irrelevant here.
        group.bench_with_input(
            BenchmarkId::new("without_plugin", &label),
            &state,
            |b, st| {
                let handler = WebUIHandler::new();
                let mut writer = BenchWriter::new(4096);
                b.iter(|| {
                    writer.clear();
                    handler
                        .handle(
                            black_box(&empty_protocol),
                            black_box(st),
                            &options,
                            &mut writer,
                        )
                        .unwrap_or_else(|error| panic!("without_plugin render failed: {error}"));
                    black_box(writer.len());
                });
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bootstrap_state_bench);
criterion_main!(benches);
