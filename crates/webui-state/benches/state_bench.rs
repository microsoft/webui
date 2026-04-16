// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use serde_json::json;
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

    // Build a state with many top-level keys
    let mut obj = serde_json::Map::with_capacity(100);
    for idx in 0..100 {
        obj.insert(format!("key_{idx}"), json!(format!("value_{idx}")));
    }
    // Add the target key at the end
    obj.insert("target".to_string(), json!("found"));
    let large_state = serde_json::Value::Object(obj);

    // Lookup early key
    group.bench_function("early_key", |b| {
        b.iter(|| find_value_by_dotted_path(black_box("key_0"), black_box(&large_state)));
    });

    // Lookup late key
    group.bench_function("late_key", |b| {
        b.iter(|| find_value_by_dotted_path(black_box("target"), black_box(&large_state)));
    });

    // Lookup middle key
    group.bench_function("middle_key", |b| {
        b.iter(|| find_value_by_dotted_path(black_box("key_50"), black_box(&large_state)));
    });

    // Missing key in large object
    group.bench_function("missing_key", |b| {
        b.iter(|| find_value_by_dotted_path(black_box("nonexistent"), black_box(&large_state)));
    });

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

criterion_group!(
    benches,
    state_path_depth_bench,
    state_value_types_bench,
    state_length_property_bench,
    state_missing_paths_bench,
    state_large_object_bench,
    state_loop_simulation_bench
);
criterion_main!(benches);
