// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#[cfg(target_os = "macos")]
use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};

#[cfg(target_os = "macos")]
fn response_buffers(c: &mut Criterion) {
    use std::hint::black_box;

    use objc2::rc::autoreleasepool;
    use objc2_foundation::NSData;

    let mut group = c.benchmark_group("macos_response_buffer");
    for size in [64 * 1024, 1024 * 1024, 16 * 1024 * 1024] {
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::new("copy", size), &size, |b, &size| {
            b.iter_batched(
                || vec![0x5a; size],
                |body| {
                    autoreleasepool(|_| {
                        black_box(NSData::with_bytes(&body));
                    })
                },
                BatchSize::PerIteration,
            );
        });
        group.bench_with_input(BenchmarkId::new("transfer", size), &size, |b, &size| {
            b.iter_batched(
                || vec![0x5a; size],
                |body| {
                    autoreleasepool(|_| {
                        black_box(NSData::from_vec(body));
                    })
                },
                BatchSize::PerIteration,
            );
        });
    }
    group.finish();
}

#[cfg(target_os = "macos")]
criterion_group!(benches, response_buffers);
#[cfg(target_os = "macos")]
criterion_main!(benches);

#[cfg(not(target_os = "macos"))]
fn main() {}
