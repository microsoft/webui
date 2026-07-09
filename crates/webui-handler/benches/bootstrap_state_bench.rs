// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Bootstrap state-serialization benchmark.
//!
//! Measures the cost of emitting the monolithic `#webui-data` bootstrap block,
//! whose `state` field serializes the ENTIRE application state as JSON on every
//! full-HTML render (`write_webui_bootstrap` → `write_script_safe_json`).
//!
//! The protocol is intentionally minimal — a bare `<body>` plus a raw
//! `body_end` signal that triggers the bootstrap emission — so the measured
//! work is dominated by state serialization (serde encode + UTF-8 revalidation
//! + `</` script-safety scan), not template rendering. Comparing the
//! `with_webui_plugin` arm (emits the block) against the `without_plugin` arm
//! (no block) isolates the serialization cost that the per-component hydration
//! redesign is intended to eliminate.
//!
//! Sizes span 64 KB / 256 KB / 1 MB of serialized state to expose the linear
//! (and, on the client, super-linear) cost that motivates the redesign. This
//! benchmark is the fixed yardstick captured BEFORE any implementation change.

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
/// plus a large array of records mixing scalar kinds (int, string, bool, float)
/// and small nested arrays. Record count is estimated up front to avoid
/// repeated serialization; the caller measures the exact serialized size for
/// throughput reporting.
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

/// Minimal full-HTML protocol that fires a single `body_end` hook.
///
/// The raw `body_end` signal resolves to no value (it is a structural hook, not
/// a state key), so it emits no text of its own but triggers the bootstrap
/// `#webui-data` block when a hydration plugin is active.
fn build_bootstrap_protocol() -> WebUIProtocol {
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
    WebUIProtocol::new(fragments)
}

fn serialized_len(state: &Value) -> usize {
    serde_json::to_vec(state)
        .map(|bytes| bytes.len())
        .unwrap_or(0)
}

fn bootstrap_state_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("bootstrap_state");
    let protocol = build_bootstrap_protocol();
    let options = RenderOptions::new("index.html", "/");

    for &target in &[64 * 1024usize, 256 * 1024, 1024 * 1024] {
        let state = build_large_state(target);
        let state_bytes = serialized_len(&state);
        let label = format!("{}KB", target / 1024);

        // Throughput is reported against the actual serialized state size so
        // results read as MB/s of state pushed through the bootstrap block.
        group.throughput(Throughput::Bytes(state_bytes as u64));

        // WITH the WebUI hydration plugin: `#webui-data` is emitted and the
        // full state is serialized via `write_script_safe_json`. This is the
        // hot path the redesign targets.
        group.bench_with_input(
            BenchmarkId::new("with_webui_plugin", &label),
            &state,
            |b, st| {
                let handler = WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()));
                let mut writer = BenchWriter::new(state_bytes + 4096);
                b.iter(|| {
                    writer.clear();
                    handler
                        .handle(black_box(&protocol), black_box(st), &options, &mut writer)
                        .unwrap_or_else(|error| panic!("render with plugin failed: {error}"));
                    black_box(writer.len());
                });
            },
        );

        // WITHOUT a plugin: no bootstrap block is emitted, so the state is
        // never serialized. Serves as the lower bound; the delta against the
        // arm above is the serialization cost the redesign removes.
        group.bench_with_input(
            BenchmarkId::new("without_plugin", &label),
            &state,
            |b, st| {
                let handler = WebUIHandler::new();
                let mut writer = BenchWriter::new(4096);
                b.iter(|| {
                    writer.clear();
                    handler
                        .handle(black_box(&protocol), black_box(st), &options, &mut writer)
                        .unwrap_or_else(|error| panic!("render without plugin failed: {error}"));
                    black_box(writer.len());
                });
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bootstrap_state_bench);
criterion_main!(benches);
