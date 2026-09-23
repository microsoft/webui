// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::hint::black_box;
use std::path::Path;
use std::sync::{Arc, RwLock};

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use serde_json::Value;

#[path = "../src/state.rs"]
mod state;

fn fixture(count: usize) -> Vec<u8> {
    let mut seed: Value = serde_json::from_str(include_str!("../../data/state.json"))
        .unwrap_or_else(|error| panic!("invalid benchmark fixture: {error}"));
    let originals = seed["contacts"]
        .as_array()
        .unwrap_or_else(|| panic!("benchmark fixture needs contacts"));
    let contacts: Vec<_> = (0..count)
        .map(|index| originals[index % originals.len()].clone())
        .collect();
    seed["contacts"] = Value::Array(contacts.clone());
    seed["filteredContacts"] = Value::Array(contacts.clone());
    seed["favoriteContacts"] = Value::Array(contacts.iter().take(count / 3).cloned().collect());
    seed["recentContacts"] = Value::Array(contacts.iter().rev().take(5).cloned().collect());
    serde_json::to_vec(&seed).unwrap_or_else(|error| panic!("cannot encode fixture: {error}"))
}

fn clone_seed(bytes: &[u8]) -> (Value, state::SharedState) {
    let value: Value = serde_json::from_slice(bytes)
        .unwrap_or_else(|error| panic!("cannot parse baseline fixture: {error}"));
    let seed = value.clone();
    (seed, Arc::new(RwLock::new(value)))
}

fn desktop_state(c: &mut Criterion) {
    let example = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../data/state.json"));
    let (_, store) = state::load_state(example)
        .unwrap_or_else(|error| panic!("cannot load example seed: {error}"));
    let stored = state::read_state(&store)
        .unwrap_or_else(|error| panic!("cannot read example store: {error}"));
    assert!(stored["contacts"].is_array());

    let mut group = c.benchmark_group("desktop_state");
    for count in [15, 1000] {
        let bytes = fixture(count);
        {
            let (_, baseline) = clone_seed(&bytes);
            let (_, canonical) = state::parse_state(&bytes)
                .unwrap_or_else(|error| panic!("cannot parse fixture: {error}"));
            let baseline = state::read_state(&baseline)
                .unwrap_or_else(|error| panic!("cannot read baseline: {error}"));
            let canonical = state::read_state(&canonical)
                .unwrap_or_else(|error| panic!("cannot read canonical store: {error}"));
            assert_eq!(baseline["contacts"], canonical["contacts"]);
            assert_eq!(baseline["groups"], canonical["groups"]);
        }
        group.throughput(Throughput::Bytes(bytes.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("cloned_seed_baseline", count),
            &bytes,
            |b, bytes| {
                b.iter(|| black_box(clone_seed(black_box(bytes))));
            },
        );
        group.bench_with_input(
            BenchmarkId::new("canonical_store", count),
            &bytes,
            |b, bytes| {
                b.iter(|| {
                    black_box(
                        state::parse_state(black_box(bytes))
                            .unwrap_or_else(|error| panic!("cannot parse fixture: {error}")),
                    )
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, desktop_state);
criterion_main!(benches);
