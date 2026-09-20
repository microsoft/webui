// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! File hashing latency, including opening/metadata, with scratch reused across events.
//! Run: cargo bench -p microsoft-webui-dev-server --bench watch_hash_bench

use std::hint::black_box;
use std::path::PathBuf;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

#[path = "../src/watch_hash.rs"]
mod watch_hash;

use watch_hash::{hash_file, HASH_BUFFER_SIZE};

fn watch_hash_bench(c: &mut Criterion) {
    let target = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    std::fs::create_dir_all(&target)
        .unwrap_or_else(|error| panic!("Cannot create benchmark fixture directory: {error}"));
    let fixtures = tempfile::Builder::new()
        .prefix("watch-hash-bench-")
        .tempdir_in(target)
        .unwrap_or_else(|error| panic!("Cannot create benchmark fixtures: {error}"));
    let mut buffer = [0_u8; HASH_BUFFER_SIZE];
    let mut group = c.benchmark_group("watch_hash");
    group
        .sample_size(50)
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3));

    for size in [1024, 1024 * 1024, 8 * 1024 * 1024] {
        let path = fixtures.path().join(format!("file-{size}"));
        std::fs::write(&path, vec![b'x'; size])
            .unwrap_or_else(|error| panic!("Cannot write benchmark fixture: {error}"));
        assert!(hash_file(&path, &mut buffer).is_some());
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::new("file_bytes", size), &path, |b, path| {
            b.iter(|| black_box(hash_file(black_box(path), &mut buffer)));
        });
    }

    for size in [1024, 256 * 1024] {
        let paths: Vec<PathBuf> = (0..32)
            .map(|index| {
                let path = fixtures.path().join(format!("burst-{size}-{index}"));
                std::fs::write(&path, vec![b'y'; size])
                    .unwrap_or_else(|error| panic!("Cannot write burst fixture: {error}"));
                path
            })
            .collect();
        group.throughput(Throughput::Bytes(32 * size as u64));
        group.bench_with_input(
            BenchmarkId::new("burst_32_files", size),
            &paths,
            |b, paths| {
                b.iter(|| {
                    for path in paths {
                        black_box(hash_file(black_box(path), &mut buffer));
                    }
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, watch_hash_bench);
criterion_main!(benches);
