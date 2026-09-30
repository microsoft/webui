// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Requested Rust heap regression for the pure PNG encoder, without AppKit.
//! Run optimized with `cargo test -p microsoft-webui-desktop --release
//! --features native-capture --test capture_allocations`.
//! Native zlib scratch, allocator overhead and RSS are not measured.

#![cfg(all(target_os = "macos", feature = "native-capture"))]
#![allow(clippy::disallowed_methods)]
// The isolated test allocator and the source-reused zlib encoder require FFI.
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

// Reuse the actual private encoder and public error/bounds, without adding a
// production test hook or loading a native window/pasteboard.
mod capture {
    pub use webui_desktop::{
        CaptureError, MAX_WEB_CAPTURE_PNG_BYTES, MAX_WEB_CAPTURE_RASTER_BYTES,
    };
}
#[path = "../src/macos/capture/png.rs"]
mod png;

#[derive(Clone, Copy, Default)]
struct Allocations {
    live: usize,
    peak: usize,
}

thread_local! {
    // Constant-initialized, allocation-free TLS isolates parallel test threads.
    static MEASURED: Cell<Option<Allocations>> = const { Cell::new(None) };
}

fn record(added: usize, removed: usize) {
    let _ = MEASURED.try_with(|measured| {
        if let Some(mut counts) = measured.get() {
            counts.live = counts.live.saturating_sub(removed) + added;
            counts.peak = counts.peak.max(counts.live);
            measured.set(Some(counts));
        }
    });
}

struct MeasuredAllocator;

// SAFETY: Every operation preserves System's pointer/layout contract. The
// accounting path only accesses allocation-free TLS, including during teardown.
unsafe impl GlobalAlloc for MeasuredAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: Forward the unchanged allocator request.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            record(layout.size(), 0);
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: Forward the unchanged allocator request.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            record(layout.size(), 0);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: Forward the caller's live allocation and original layout.
        unsafe { System.dealloc(ptr, layout) };
        record(0, layout.size());
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: Forward the original allocation and requested new size.
        let next = unsafe { System.realloc(ptr, layout, new_size) };
        if !next.is_null() {
            // Requested live sizes, not System's internal relocation overlap.
            record(new_size, layout.size());
        }
        next
    }
}

#[global_allocator]
static ALLOCATOR: MeasuredAllocator = MeasuredAllocator;

struct Measurement;

impl Drop for Measurement {
    fn drop(&mut self) {
        MEASURED.with(|measured| measured.set(None));
    }
}

#[test]
fn maximum_noise_encoding_does_not_overlap_scanlines_with_final_png() {
    let mut pixels = vec![0; capture::MAX_WEB_CAPTURE_RASTER_BYTES];
    let mut seed = 0x1234_5678_u32;
    for pixel in pixels.as_chunks_mut::<4>().0 {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        pixel[..3].copy_from_slice(&seed.to_le_bytes()[..3]);
        pixel[3] = 255;
    }
    MEASURED.with(|measured| measured.set(Some(Allocations::default())));
    let measurement = Measurement;
    let encoded = png::encode_rgba(&pixels, 1600, 1200, capture::MAX_WEB_CAPTURE_PNG_BYTES);
    let counts = MEASURED.with(|measured| measured.get().unwrap());
    drop(measurement);
    let encoded = encoded.unwrap();
    assert!(encoded.len() > 5 * 1024 * 1024);
    assert_eq!(counts.live, encoded.capacity());

    // Two maximum filtered-row-sized allocations plus conservative zlib/PNG
    // framing slack. A third simultaneous full PNG must not fit this budget.
    // Keep slack instead of pinning zlib's exact output or allocator layout.
    let filtered_rows = (1600 * 4 + 1) * 1200;
    let max_live = filtered_rows * 2 + 64 * 1024;
    assert!(
        counts.peak <= max_live,
        "encoder requested heap peak {} exceeds {max_live}",
        counts.peak
    );
}
