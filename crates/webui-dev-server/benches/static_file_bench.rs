// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Static-file request latency, including blocking-pool scheduling and filesystem I/O.
//! Run: cargo bench -p microsoft-webui-dev-server --bench static_file_bench

use std::hint::black_box;
use std::path::PathBuf;
use std::time::Duration;

use actix_web::body::to_bytes;
use actix_web::http::StatusCode;
use actix_web::test::TestRequest;
use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use webui_dev_server::{serve_static_file, LiveReload, NotFoundStrategy, StaticServeConfig};

const FIXTURE_BYTES: usize = 4 * 1024;

fn config(root: PathBuf, not_found: NotFoundStrategy) -> StaticServeConfig {
    StaticServeConfig {
        root,
        base_path: "/".to_owned(),
        livereload: LiveReload::new("/__bench/livereload"),
        not_found,
    }
}

fn assert_response(
    runtime: &tokio::runtime::Runtime,
    request: &actix_web::HttpRequest,
    config: &StaticServeConfig,
    expected_status: StatusCode,
) {
    let response = runtime.block_on(serve_static_file(request, config));
    assert_eq!(response.status(), expected_status);
    let body = runtime
        .block_on(to_bytes(response.into_body()))
        .unwrap_or_else(|error| panic!("benchmark response body failed: {error}"));
    assert_eq!(body.len(), FIXTURE_BYTES);
}

fn static_file_bench(c: &mut Criterion) {
    let target = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    std::fs::create_dir_all(&target)
        .unwrap_or_else(|error| panic!("Cannot create benchmark fixture directory: {error}"));
    let fixtures = tempfile::Builder::new()
        .prefix("static-file-bench-")
        .tempdir_in(target)
        .unwrap_or_else(|error| panic!("Cannot create benchmark fixtures: {error}"));
    std::fs::write(fixtures.path().join("asset.js"), vec![b'x'; FIXTURE_BYTES])
        .unwrap_or_else(|error| panic!("Cannot write benchmark asset: {error}"));
    std::fs::write(fixtures.path().join("404.js"), vec![b'y'; FIXTURE_BYTES])
        .unwrap_or_else(|error| panic!("Cannot write benchmark fallback: {error}"));

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap_or_else(|error| panic!("Cannot build benchmark runtime: {error}"));
    let hit_request = TestRequest::with_uri("/asset.js").to_http_request();
    let miss_request = TestRequest::with_uri("/missing.js").to_http_request();
    let hit_config = config(fixtures.path().to_path_buf(), NotFoundStrategy::Plain);
    let miss_config = config(
        fixtures.path().to_path_buf(),
        NotFoundStrategy::File(PathBuf::from("404.js")),
    );

    assert_response(&runtime, &hit_request, &hit_config, StatusCode::OK);
    assert_response(&runtime, &miss_request, &miss_config, StatusCode::NOT_FOUND);

    let mut group = c.benchmark_group("static_file_request");
    group
        .sample_size(100)
        .warm_up_time(Duration::from_secs(2))
        .measurement_time(Duration::from_secs(5))
        .throughput(Throughput::Elements(1));

    group.bench_function("hit_4k", |b| {
        b.iter(|| {
            black_box(runtime.block_on(serve_static_file(
                black_box(&hit_request),
                black_box(&hit_config),
            )));
        });
    });
    group.bench_function("custom_404_4k", |b| {
        b.iter(|| {
            black_box(runtime.block_on(serve_static_file(
                black_box(&miss_request),
                black_box(&miss_config),
            )));
        });
    });
    group.finish();
}

criterion_group!(benches, static_file_bench);
criterion_main!(benches);
