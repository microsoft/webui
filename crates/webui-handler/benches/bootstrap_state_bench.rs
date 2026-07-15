// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Bootstrap state-serialization benchmark.
//!
//! Measures the cost of emitting the `#webui-data` bootstrap block, whose
//! `state` field is serialized on every full-HTML render
//! (`write_webui_bootstrap` → `write_selected_state`).
//!
//! The projected-hydration design filters the SSR state down to only the keys
//! authored client components actually hydrate before
//! serializing it through the script-safe JSON writer. This benchmark pins
//! that behavior with seven arms per size:
//!
//! * `hydratable_collection` — hydration keys contain every top-level key, including
//!   the large `items` collection. This models a real `<for>` root and is the
//!   equal-byte anti-regression guard.
//! * `uncertain_full_state` — analysis selected `All`, so the handler bypasses
//!   key collection and preserves the complete state.
//! * `server_only_collection` — hydration keys contain only the small metadata keys,
//!   so the large `items` array is projected away. This models a large
//!   render-only/server-only collection, not a hydrated `<for>` root.
//! * `authored_navigation_only_component` — an authored component retains the
//!   full template-root navigation surface but declares no JavaScript-owned
//!   hydration fields. This is the property-level split exercised by authored
//!   event/ref components such as the routes example shell.
//! * `dormant_scriptless_component` — the reachable component retains browser
//!   template metadata and navigation keys, but its initial hydration key set is
//!   empty, so the state is never traversed.
//! * `missing_component_metadata` — intentionally missing component
//!   metadata. Runtime safety must preserve full state instead of treating
//!   missing metadata as a proven-empty surface.
//! * `without_plugin` — no bootstrap block is emitted at all; the structural
//!   lower bound.
//!
//! Separate groups cover a very wide top-level object and a routed application
//! whose inactive route owns the large collection. Those cases gate adaptive
//! projection lookup and request-scoped hydration key collection.
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
use webui_handler::{PreparedProtocol, RenderOptions, ResponseWriter, WebUIHandler};
use webui_protocol::{
    ComponentData, FragmentList, InitialStateStrategy, StateProjectionMode, WebUIFragment,
    WebUIFragmentRoute, WebUIProtocol,
};

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

fn build_wide_state(keys: usize) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("keptA".to_string(), json!("a"));
    map.insert("keptB".to_string(), json!("b"));
    for idx in 0..keys {
        map.insert(format!("serverOnly{idx:05}"), json!(idx));
    }
    Value::Object(map)
}

fn build_routed_state(contact_count: usize) -> Value {
    let mut contacts = Vec::with_capacity(contact_count);
    for idx in 0..contact_count {
        contacts.push(json!({
            "id": idx,
            "name": format!("Contact {idx:05}"),
            "email": format!("contact{idx:05}@example.com"),
            "company": "Contoso",
            "phone": "+1-555-0100",
        }));
    }
    let recent_contacts = contacts.iter().take(5).cloned().collect::<Vec<_>>();
    json!({
        "page": "dashboard",
        "contacts": contacts,
        "recentContacts": recent_contacts,
    })
}

/// Every top-level key of [`build_large_state`], sorted. Projecting against
/// this emits the full state (worst case / anti-regression guard).
fn full_hydration_keys() -> Vec<String> {
    let mut keys = vec![
        "count".to_string(),
        "generatedAt".to_string(),
        "items".to_string(),
        "title".to_string(),
    ];
    keys.sort();
    keys
}

/// The small metadata keys only, sorted — the large `items` array is
/// deliberately excluded so projection collapses the payload (typical case).
fn server_only_hydration_keys() -> Vec<String> {
    let mut keys = vec![
        "count".to_string(),
        "generatedAt".to_string(),
        "title".to_string(),
    ];
    keys.sort();
    keys
}

fn build_partial_protocol(
    hydration_keys: &[&str],
    navigation_keys: &[&str],
    compiler_owned: bool,
    navigation_mode: Option<StateProjectionMode>,
) -> PreparedProtocol {
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::route("/", "benchmark-page")],
        },
    );
    fragments.insert(
        "benchmark-page".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::raw("<p>Benchmark</p>")],
        },
    );
    let mut protocol = WebUIProtocol::new(fragments);
    let template_json = if compiler_owned {
        r#"{"h":"<p>Benchmark</p>","th":1}"#
    } else {
        r#"{"h":"<p>Benchmark</p>"}"#
    };
    protocol.components.insert(
        "benchmark-page".to_string(),
        ComponentData {
            template_json: template_json.to_string(),
            hydration_mode: if hydration_keys.is_empty() && !compiler_owned {
                StateProjectionMode::Keys as i32
            } else {
                keyed_mode(hydration_keys)
            },
            hydration_keys: hydration_keys
                .iter()
                .map(|key| (*key).to_string())
                .collect(),
            navigation_mode: navigation_mode
                .map_or_else(|| keyed_mode(navigation_keys), |mode| mode as i32),
            navigation_keys: navigation_keys
                .iter()
                .map(|key| (*key).to_string())
                .collect(),
            ..Default::default()
        },
    );
    PreparedProtocol::new(protocol)
}

fn build_routed_protocol() -> WebUIProtocol {
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("<!DOCTYPE html><html><body>"),
                WebUIFragment::route_from(WebUIFragmentRoute {
                    path: "/".to_string(),
                    fragment_id: "dashboard-page".to_string(),
                    exact: true,
                    ..Default::default()
                }),
                WebUIFragment::route_from(WebUIFragmentRoute {
                    path: "/contacts".to_string(),
                    fragment_id: "contacts-page".to_string(),
                    exact: true,
                    ..Default::default()
                }),
                WebUIFragment::signal("body_end", true),
                WebUIFragment::raw("</body></html>"),
            ],
        },
    );
    fragments.insert(
        "dashboard-page".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::raw("<p>Dashboard</p>")],
        },
    );
    fragments.insert(
        "contacts-page".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::raw("<p>Contacts</p>")],
        },
    );

    let mut protocol = WebUIProtocol::new(fragments);
    protocol.initial_state_strategy = InitialStateStrategy::Components as i32;
    protocol.components.insert(
        "dashboard-page".to_string(),
        ComponentData {
            template_json: "{}".to_string(),
            hydration_mode: StateProjectionMode::Keys as i32,
            hydration_keys: vec!["page".to_string(), "recentContacts".to_string()],
            ..Default::default()
        },
    );
    protocol.components.insert(
        "contacts-page".to_string(),
        ComponentData {
            template_json: "{}".to_string(),
            hydration_mode: StateProjectionMode::Keys as i32,
            hydration_keys: vec!["contacts".to_string(), "page".to_string()],
            ..Default::default()
        },
    );
    protocol
}

/// Minimal full-HTML protocol that fires a single `body_end` hook, carrying the
/// supplied authored-component hydration keys used to project the state at
/// emission time. A compiler-owned component keeps template metadata and
/// navigation keys while its initial hydration key set stays empty.
///
/// The raw `body_end` signal resolves to no value (it is a structural hook, not
/// a state key), so it emits no text of its own but triggers the bootstrap
/// `#webui-data` block when a hydration plugin is active.
fn build_bootstrap_protocol(
    hydration_keys: Vec<String>,
    navigation_keys: Vec<String>,
    compiler_owned: bool,
) -> WebUIProtocol {
    let mut fragments = HashMap::new();
    fragments.insert(
        "index.html".to_string(),
        FragmentList {
            fragments: vec![
                WebUIFragment::raw("<!DOCTYPE html><html><body>"),
                WebUIFragment::component("bench-component"),
                WebUIFragment::signal("body_end", true),
                WebUIFragment::raw("</body></html>"),
            ],
        },
    );
    fragments.insert(
        "bench-component".to_string(),
        FragmentList {
            fragments: vec![WebUIFragment::raw("<p>ready</p>")],
        },
    );
    let mut protocol = WebUIProtocol::new(fragments);
    protocol.initial_state_strategy = InitialStateStrategy::Components as i32;
    let template_json = if compiler_owned {
        r#"{"h":"<p>ready</p>","tr":["count","generatedAt","items","title"],"th":1}"#
    } else {
        r#"{"h":"<p>ready</p>"}"#
    };
    protocol.components.insert(
        "bench-component".to_string(),
        ComponentData {
            template_json: template_json.to_string(),
            hydration_mode: if hydration_keys.is_empty() && !compiler_owned {
                StateProjectionMode::Keys as i32
            } else {
                keyed_mode(&hydration_keys)
            },
            hydration_keys,
            navigation_mode: keyed_mode(&navigation_keys),
            navigation_keys,
            ..Default::default()
        },
    );
    protocol
}

fn keyed_mode<T>(keys: &[T]) -> i32 {
    if keys.is_empty() {
        StateProjectionMode::None as i32
    } else {
        StateProjectionMode::Keys as i32
    }
}

fn build_missing_metadata_bootstrap_protocol() -> WebUIProtocol {
    let mut protocol = build_bootstrap_protocol(Vec::new(), Vec::new(), true);
    protocol.components.clear();
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

    // Protocols differ only by the projection keys; state is shared per size.
    let full_keys = full_hydration_keys();
    let metadata_keys = server_only_hydration_keys();
    let full_protocol = build_bootstrap_protocol(full_keys.clone(), full_keys.clone(), false);
    let mut full_fallback_protocol = build_bootstrap_protocol(Vec::new(), Vec::new(), false);
    let fallback_component = full_fallback_protocol
        .components
        .get_mut("bench-component")
        .unwrap_or_else(|| panic!("benchmark component missing"));
    fallback_component.hydration_mode = StateProjectionMode::All as i32;
    fallback_component.navigation_mode = StateProjectionMode::All as i32;
    let server_only_protocol =
        build_bootstrap_protocol(metadata_keys.clone(), metadata_keys, false);
    let authored_navigation_only_protocol =
        build_bootstrap_protocol(Vec::new(), full_keys.clone(), false);
    let dormant_protocol = build_bootstrap_protocol(Vec::new(), full_keys, true);
    let missing_metadata_protocol = build_missing_metadata_bootstrap_protocol();

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
            BenchmarkId::new("hydratable_collection", &label),
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
                        .unwrap_or_else(|error| {
                            panic!("hydratable_collection render failed: {error}")
                        });
                    black_box(writer.len());
                });
            },
        );

        // UNKNOWN surface: preserve full state without collecting or searching
        // keys. This is the correctness fallback for unsupported source forms.
        group.bench_with_input(
            BenchmarkId::new("uncertain_full_state", &label),
            &state,
            |b, st| {
                let handler = WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()));
                let mut writer = BenchWriter::new(state_bytes + 4096);
                b.iter(|| {
                    writer.clear();
                    handler
                        .handle(
                            black_box(&full_fallback_protocol),
                            black_box(st),
                            &options,
                            &mut writer,
                        )
                        .unwrap_or_else(|error| {
                            panic!("uncertain_full_state render failed: {error}")
                        });
                    black_box(writer.len());
                });
            },
        );

        // TYPICAL projection: only the small metadata keys are hydratable, so
        // the large `items` array is projected away and the emitted block stays
        // tiny regardless of total state size. This is the redesign's win.
        group.bench_with_input(
            BenchmarkId::new("server_only_collection", &label),
            &state,
            |b, st| {
                let handler = WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()));
                let mut writer = BenchWriter::new(4096);
                b.iter(|| {
                    writer.clear();
                    handler
                        .handle(
                            black_box(&server_only_protocol),
                            black_box(st),
                            &options,
                            &mut writer,
                        )
                        .unwrap_or_else(|error| {
                            panic!("server_only_collection render failed: {error}")
                        });
                    black_box(writer.len());
                });
            },
        );

        // AUTHORED NAVIGATION-ONLY component: event/ref hydration remains active,
        // but template roots stay out of initial state because SSR already
        // rendered them. Partial navigation still retains the full key surface.
        group.bench_with_input(
            BenchmarkId::new("authored_navigation_only_component", &label),
            &state,
            |b, st| {
                let handler = WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()));
                let mut writer = BenchWriter::new(4096);
                b.iter(|| {
                    writer.clear();
                    handler
                        .handle(
                            black_box(&authored_navigation_only_protocol),
                            black_box(st),
                            &options,
                            &mut writer,
                        )
                        .unwrap_or_else(|error| {
                            panic!("authored_navigation_only_component render failed: {error}")
                        });
                    black_box(writer.len());
                });
            },
        );

        // DORMANT SCRIPTLESS component: browser template metadata is emitted,
        // but no initial state keys are projected or serialized.
        group.bench_with_input(
            BenchmarkId::new("dormant_scriptless_component", &label),
            &state,
            |b, st| {
                let handler = WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()));
                let mut writer = BenchWriter::new(4096);
                b.iter(|| {
                    writer.clear();
                    handler
                        .handle(
                            black_box(&dormant_protocol),
                            black_box(st),
                            &options,
                            &mut writer,
                        )
                        .unwrap_or_else(|error| {
                            panic!("dormant_scriptless_component render failed: {error}")
                        });
                    black_box(writer.len());
                });
            },
        );

        // MISSING metadata: runtime safety falls back to full state instead of
        // silently treating an unknown component surface as empty.
        group.bench_with_input(
            BenchmarkId::new("missing_component_metadata", &label),
            &state,
            |b, st| {
                let handler = WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()));
                let mut writer = BenchWriter::new(4096);
                b.iter(|| {
                    writer.clear();
                    handler
                        .handle(
                            black_box(&missing_metadata_protocol),
                            black_box(st),
                            &options,
                            &mut writer,
                        )
                        .unwrap_or_else(|error| {
                            panic!("missing_component_metadata render failed: {error}")
                        });
                    black_box(writer.len());
                });
            },
        );

        // WITHOUT a plugin: no bootstrap block is emitted, so the state is
        // never touched. Structural lower bound. `empty_protocol` carries an
        // empty startup key set, but with no plugin active no block is emitted, so
        // those keys are irrelevant here.
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
                            black_box(&dormant_protocol),
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

    let wide_state = build_wide_state(30_000);
    let wide_state_bytes = serialized_len(&wide_state);
    let wide_keys = vec!["keptA".to_string(), "keptB".to_string()];
    let wide_protocol = build_bootstrap_protocol(wide_keys.clone(), wide_keys, false);
    let mut wide_group = c.benchmark_group("bootstrap_state_wide");
    wide_group.throughput(Throughput::Bytes(wide_state_bytes as u64));
    wide_group.bench_function("sparse_keys_30000", |b| {
        let handler = WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()));
        let mut writer = BenchWriter::new(4096);
        b.iter(|| {
            writer.clear();
            handler
                .handle(
                    black_box(&wide_protocol),
                    black_box(&wide_state),
                    &options,
                    &mut writer,
                )
                .unwrap_or_else(|error| panic!("wide-state render failed: {error}"));
            black_box(writer.len());
        });
    });
    wide_group.finish();

    let routed_state = build_routed_state(1_000);
    let routed_state_bytes = serialized_len(&routed_state);
    let routed_protocol = build_routed_protocol();
    let mut dormant_routed_protocol = build_routed_protocol();
    for component in dormant_routed_protocol.components.values_mut() {
        component.navigation_keys = component.hydration_keys.clone();
        component.navigation_mode = component.hydration_mode;
        component.hydration_keys.clear();
        component.hydration_mode = StateProjectionMode::None as i32;
        component.template_json = r#"{"h":"<p>ready</p>","th":1}"#.to_string();
    }
    let mut missing_metadata_routed_protocol = dormant_routed_protocol.clone();
    missing_metadata_routed_protocol.components.clear();
    let mut route_group = c.benchmark_group("bootstrap_state_route");
    route_group.throughput(Throughput::Bytes(routed_state_bytes as u64));
    for &(name, path, protocol) in &[
        ("dashboard_excludes_contacts", "/", &routed_protocol),
        ("contacts_includes_contacts", "/contacts", &routed_protocol),
        (
            "contacts_dormant_component",
            "/contacts",
            &dormant_routed_protocol,
        ),
        (
            "contacts_missing_component_metadata",
            "/contacts",
            &missing_metadata_routed_protocol,
        ),
    ] {
        route_group.bench_function(name, |b| {
            let handler = WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()));
            let render_options = RenderOptions::new("index.html", path);
            let mut writer = BenchWriter::new(routed_state_bytes + 4096);
            b.iter(|| {
                writer.clear();
                handler
                    .handle(
                        black_box(protocol),
                        black_box(&routed_state),
                        &render_options,
                        &mut writer,
                    )
                    .unwrap_or_else(|error| panic!("route-state render failed: {error}"));
                black_box(writer.len());
            });
        });
    }
    route_group.finish();
}

fn partial_state_serialization_bench(c: &mut Criterion) {
    let response = json!({
        "templates": {},
        "templateFunctions": {},
        "templateStyles": [],
        "cssHrefs": [],
        "inventory": "",
        "path": "/",
        "chain": [],
    });
    let projected_protocol = build_partial_protocol(
        &["count", "generatedAt", "title"],
        &["count", "generatedAt", "title"],
        false,
        None,
    );
    let scriptless_protocol =
        build_partial_protocol(&[], &["count", "generatedAt", "title"], true, None);
    let static_protocol = build_partial_protocol(&[], &[], true, None);
    let full_protocol = build_partial_protocol(&[], &[], false, Some(StateProjectionMode::All));
    let mut group = c.benchmark_group("partial_state_serialization");

    for &target in &[64 * 1024usize, 1024 * 1024] {
        let state = build_large_state(target);
        let state_json = serde_json::to_string(&state)
            .unwrap_or_else(|error| panic!("state serialization failed: {error}"));
        let label = format!("{}KB", target / 1024);
        group.throughput(Throughput::Bytes(state_json.len() as u64));

        group.bench_with_input(
            BenchmarkId::new("legacy_parse_and_reserialize", &label),
            &state_json,
            |b, input| {
                b.iter(|| {
                    let state: Value = serde_json::from_str(black_box(input))
                        .unwrap_or_else(|error| panic!("state parse failed: {error}"));
                    let mut result = response.clone();
                    result
                        .as_object_mut()
                        .unwrap_or_else(|| panic!("benchmark response must be an object"))
                        .insert("state".to_string(), state);
                    let output = serde_json::to_string(&result)
                        .unwrap_or_else(|error| panic!("response serialization failed: {error}"));
                    black_box(output.len());
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("explicit_full_state", &label),
            &state_json,
            |b, input| {
                b.iter(|| {
                    let output = webui_handler::route_handler::render_partial_prepared(
                        &full_protocol,
                        black_box(input),
                        "index.html",
                        "/",
                        "",
                    )
                    .unwrap_or_else(|error| {
                        panic!("full partial response serialization failed: {error}")
                    });
                    black_box(output.len());
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("projected_authored_state", &label),
            &state_json,
            |b, input| {
                b.iter(|| {
                    let output = webui_handler::route_handler::render_partial_prepared(
                        &projected_protocol,
                        black_box(input),
                        "index.html",
                        "/",
                        "",
                    )
                    .unwrap_or_else(|error| {
                        panic!("projected partial response serialization failed: {error}")
                    });
                    black_box(output.len());
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("projected_scriptless_state", &label),
            &state_json,
            |b, input| {
                b.iter(|| {
                    let output = webui_handler::route_handler::render_partial_prepared(
                        &scriptless_protocol,
                        black_box(input),
                        "index.html",
                        "/",
                        "",
                    )
                    .unwrap_or_else(|error| {
                        panic!("scriptless partial response serialization failed: {error}")
                    });
                    black_box(output.len());
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("static_scriptless_state", &label),
            &state_json,
            |b, input| {
                b.iter(|| {
                    let output = webui_handler::route_handler::render_partial_prepared(
                        &static_protocol,
                        black_box(input),
                        "index.html",
                        "/",
                        "",
                    )
                    .unwrap_or_else(|error| {
                        panic!("static partial response serialization failed: {error}")
                    });
                    black_box(output.len());
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bootstrap_state_bench,
    partial_state_serialization_bench
);
criterion_main!(benches);
