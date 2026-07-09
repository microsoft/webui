// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use serde_json::json;
use serde_json::Value;
use std::hint::black_box;
use webui_state::find_value_by_dotted_path;

fn state_path_depth_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("state_path_depth");

    // Flat: 1 level
    let flat_state = json!({"name": "Alice"});
    group.bench_function("depth_1", |b| {
        b.iter(|| find_value_by_dotted_path(black_box("name"), black_box(&flat_state)));
    });

    // Shallow: 2 levels
    let shallow_state = json!({"user": {"name": "Alice"}});
    group.bench_function("depth_2", |b| {
        b.iter(|| find_value_by_dotted_path(black_box("user.name"), black_box(&shallow_state)));
    });

    // Medium: 3 levels (common in real apps: item.property.value)
    let medium_state = json!({"user": {"profile": {"name": "Alice"}}});
    group.bench_function("depth_3", |b| {
        b.iter(|| {
            find_value_by_dotted_path(black_box("user.profile.name"), black_box(&medium_state))
        });
    });

    // Deep: 5 levels
    let deep_state = json!({"a": {"b": {"c": {"d": {"name": "Alice"}}}}});
    group.bench_function("depth_5", |b| {
        b.iter(|| find_value_by_dotted_path(black_box("a.b.c.d.name"), black_box(&deep_state)));
    });

    // Very deep: 8 levels
    let very_deep_state = json!({"a": {"b": {"c": {"d": {"e": {"f": {"g": {"name": "Alice"}}}}}}}});
    group.bench_function("depth_8", |b| {
        b.iter(|| {
            find_value_by_dotted_path(black_box("a.b.c.d.e.f.g.name"), black_box(&very_deep_state))
        });
    });

    group.finish();
}

fn state_value_types_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("state_value_types");

    let state = json!({
        "str_val": "hello world",
        "num_val": 42,
        "bool_val": true,
        "null_val": null,
        "arr_val": [1, 2, 3, 4, 5],
        "obj_val": {"key": "value", "nested": {"deep": true}},
    });

    // String value (most common in templates)
    group.bench_function("string", |b| {
        b.iter(|| find_value_by_dotted_path(black_box("str_val"), black_box(&state)));
    });

    // Number value
    group.bench_function("number", |b| {
        b.iter(|| find_value_by_dotted_path(black_box("num_val"), black_box(&state)));
    });

    // Boolean value
    group.bench_function("boolean", |b| {
        b.iter(|| find_value_by_dotted_path(black_box("bool_val"), black_box(&state)));
    });

    // Array value (clone cost)
    group.bench_function("array", |b| {
        b.iter(|| find_value_by_dotted_path(black_box("arr_val"), black_box(&state)));
    });

    // Object value (clone cost)
    group.bench_function("object", |b| {
        b.iter(|| find_value_by_dotted_path(black_box("obj_val"), black_box(&state)));
    });

    group.finish();
}

fn state_length_property_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("state_length_property");

    // Array .length — common in templates for "{{items.length}} items"
    let arr_state = json!({"items": [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]});
    group.bench_function("array_length", |b| {
        b.iter(|| find_value_by_dotted_path(black_box("items.length"), black_box(&arr_state)));
    });

    // String .length
    let str_state = json!({"title": "Hello World"});
    group.bench_function("string_length", |b| {
        b.iter(|| find_value_by_dotted_path(black_box("title.length"), black_box(&str_state)));
    });

    // Nested array .length
    let nested_state = json!({"data": {"results": [1, 2, 3]}});
    group.bench_function("nested_array_length", |b| {
        b.iter(|| {
            find_value_by_dotted_path(black_box("data.results.length"), black_box(&nested_state))
        });
    });

    group.finish();
}

fn state_missing_paths_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("state_missing_paths");

    let state = json!({
        "user": {
            "profile": {
                "name": "Alice"
            }
        }
    });

    // Miss at first level — fast return
    group.bench_function("miss_first_level", |b| {
        b.iter(|| find_value_by_dotted_path(black_box("nonexistent"), black_box(&state)));
    });

    // Miss at second level
    group.bench_function("miss_second_level", |b| {
        b.iter(|| find_value_by_dotted_path(black_box("user.missing"), black_box(&state)));
    });

    // Miss at third level (after successful traversal)
    group.bench_function("miss_third_level", |b| {
        b.iter(|| find_value_by_dotted_path(black_box("user.profile.missing"), black_box(&state)));
    });

    // Path through scalar — trying to traverse into a string
    group.bench_function("through_scalar", |b| {
        b.iter(|| {
            find_value_by_dotted_path(black_box("user.profile.name.first"), black_box(&state))
        });
    });

    group.finish();
}

fn state_large_object_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("state_large_object");

    // Sweep object *width* to expose serde_json's BTreeMap O(log n) key lookup
    // as top-level state grows. 100 → 10k models small pages up to very large
    // dashboards / API payloads splatted into state.
    for &width in &[100usize, 1_000, 10_000] {
        let mut obj = serde_json::Map::with_capacity(width + 1);
        for idx in 0..width {
            obj.insert(format!("key_{idx}"), json!(format!("value_{idx}")));
        }
        // Add the target key at the end.
        obj.insert("target".to_string(), json!("found"));
        let large_state = Value::Object(obj);

        // Lookup early key.
        group.bench_with_input(
            BenchmarkId::new("early_key", width),
            &large_state,
            |b, st| {
                b.iter(|| find_value_by_dotted_path(black_box("key_0"), black_box(st)));
            },
        );

        // Lookup middle key.
        let middle_key = format!("key_{}", width / 2);
        group.bench_with_input(
            BenchmarkId::new("middle_key", width),
            &large_state,
            |b, st| {
                b.iter(|| find_value_by_dotted_path(black_box(middle_key.as_str()), black_box(st)));
            },
        );

        // Lookup late key.
        group.bench_with_input(
            BenchmarkId::new("late_key", width),
            &large_state,
            |b, st| {
                b.iter(|| find_value_by_dotted_path(black_box("target"), black_box(st)));
            },
        );

        // Missing key in large object.
        group.bench_with_input(
            BenchmarkId::new("missing_key", width),
            &large_state,
            |b, st| {
                b.iter(|| find_value_by_dotted_path(black_box("nonexistent"), black_box(st)));
            },
        );
    }

    group.finish();
}

fn state_loop_simulation_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("state_loop_simulation");

    // Simulate what the handler does in a for-loop: repeated lookups on per-item state
    let items: Vec<serde_json::Value> = (0..100)
        .map(|i| {
            json!({
                "id": i,
                "name": format!("Item {i}"),
                "enabled": i % 2 == 0
            })
        })
        .collect();

    // Simulate looking up item.name for each item (like the handler does)
    group.bench_with_input(BenchmarkId::new("item_lookup", 100), &items, |b, items| {
        b.iter(|| {
            for item in items.iter() {
                let _ = find_value_by_dotted_path(black_box("name"), black_box(item));
            }
        });
    });

    // Simulate looking up nested property per item
    let nested_items: Vec<serde_json::Value> = (0..100)
        .map(|i| {
            json!({
                "data": {
                    "label": format!("Label {i}")
                }
            })
        })
        .collect();

    group.bench_with_input(
        BenchmarkId::new("nested_item_lookup", 100),
        &nested_items,
        |b, items| {
            b.iter(|| {
                for item in items.iter() {
                    let _ = find_value_by_dotted_path(black_box("data.label"), black_box(item));
                }
            });
        },
    );

    group.finish();
}

// ── Large-JSON parse benchmarks ──────────────────────────────────────────
//
// The Node/WASM hosts call `serde_json::from_str::<Value>` on the state object
// for *every* render request. Traces on the SSR pipeline point at this parse
// as the dominant per-request CPU cost, so these benches measure it directly as
// state grows in width, depth, and array size. Throughput is reported in bytes
// so results read as JSON parse MB/s.

/// Wide flat object: `{"key_0": "value_0", ...}` — a large bag of top-level fields.
fn wide_flat_json(width: usize) -> String {
    let mut obj = serde_json::Map::with_capacity(width);
    for idx in 0..width {
        obj.insert(format!("key_{idx}"), json!(format!("value_{idx}")));
    }
    serde_json::to_string(&Value::Object(obj)).unwrap_or_else(|e| panic!("serialize failed: {e}"))
}

/// Array-of-objects catalog: `{"items": [{id, name, price, ...}, ...]}` — models
/// the list/table-heavy pages that carry the largest state payloads.
fn catalog_json(item_count: usize) -> String {
    let items: Vec<Value> = (0..item_count)
        .map(|idx| {
            json!({
                "id": idx,
                "name": format!("Product {idx}"),
                "price": idx * 3 + 99,
                "inStock": idx % 2 == 0,
                "tags": ["new", "sale", "featured"],
                "description": "A high quality product with a reasonably long description.",
            })
        })
        .collect();
    serde_json::to_string(&json!({ "items": items }))
        .unwrap_or_else(|e| panic!("serialize failed: {e}"))
}

/// Deeply nested object: `{"child": {"child": {... {"name": "Alice"}}}}`.
fn deep_json(depth: usize) -> String {
    let mut value = json!({ "name": "Alice" });
    for _ in 0..depth {
        value = json!({ "child": value });
    }
    serde_json::to_string(&value).unwrap_or_else(|e| panic!("serialize failed: {e}"))
}

fn state_parse_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("state_parse");

    // Wide flat objects — allocation-per-key + BTreeMap insertion cost.
    for &width in &[100usize, 1_000, 10_000] {
        let source = wide_flat_json(width);
        group.throughput(Throughput::Bytes(source.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("wide_flat_keys", width),
            &source,
            |b, s| {
                b.iter(|| {
                    serde_json::from_str::<Value>(black_box(s))
                        .unwrap_or_else(|e| panic!("parse failed: {e}"))
                });
            },
        );
    }

    // Array-of-objects catalogs — the realistic large-state shape.
    for &item_count in &[100usize, 1_000, 10_000] {
        let source = catalog_json(item_count);
        group.throughput(Throughput::Bytes(source.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("catalog_items", item_count),
            &source,
            |b, s| {
                b.iter(|| {
                    serde_json::from_str::<Value>(black_box(s))
                        .unwrap_or_else(|e| panic!("parse failed: {e}"))
                });
            },
        );
    }

    // Deeply nested objects — traversal/recursion-depth cost in the parser.
    // Capped below serde_json's hard 128-level recursion limit (a depth-128
    // state actually fails to parse — a real constraint for host state shape).
    for &depth in &[8usize, 32, 64] {
        let source = deep_json(depth);
        group.throughput(Throughput::Bytes(source.len() as u64));
        group.bench_with_input(BenchmarkId::new("deep_nesting", depth), &source, |b, s| {
            b.iter(|| {
                serde_json::from_str::<Value>(black_box(s))
                    .unwrap_or_else(|e| panic!("parse failed: {e}"))
            });
        });
    }

    group.finish();
}

fn state_parse_and_lookup_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("state_parse_and_lookup");

    // Models the true per-request cost: parse the state, then resolve a signal
    // from it. The gap versus `state_parse` alone is the lookup share; the gap
    // versus `state_path_depth` is the parse share.
    for &item_count in &[1_000usize, 10_000] {
        let source = catalog_json(item_count);
        group.throughput(Throughput::Bytes(source.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("catalog_items", item_count),
            &source,
            |b, s| {
                b.iter(|| {
                    let state: Value = serde_json::from_str(black_box(s))
                        .unwrap_or_else(|e| panic!("parse failed: {e}"));
                    black_box(find_value_by_dotted_path(
                        black_box("items.length"),
                        black_box(&state),
                    ))
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    state_path_depth_bench,
    state_value_types_bench,
    state_length_property_bench,
    state_missing_paths_bench,
    state_large_object_bench,
    state_loop_simulation_bench,
    state_parse_bench,
    state_parse_and_lookup_bench
);
criterion_main!(benches);
