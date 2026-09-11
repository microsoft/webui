// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

use super::*;
use tokio::sync::mpsc::channel;

#[test]
fn pool_reuses_normal_buffer_without_retaining_content() {
    let pool = ChunkPool::new(2, 64);
    let mut buf = pool.acquire();
    let allocation = buf.as_ptr();
    buf.extend_from_slice(b"previous response");
    pool.release(buf);
    assert_eq!(pool.idle_count(), 1);

    let reused = pool.acquire();
    assert!(reused.is_empty());
    assert_eq!(reused.as_ptr(), allocation);
    assert_eq!(reused.capacity(), 64);
    assert_eq!(pool.idle_count(), 0);
}

#[test]
fn pool_rejects_oversized_capacity_instead_of_shrinking() {
    let pool = ChunkPool::new(4, 4096);
    for capacity in [4097, 64 * 1024] {
        let mut buf = Vec::with_capacity(capacity);
        buf.extend_from_slice(b"small payload");
        pool.release(buf);
        assert_eq!(pool.idle_count(), 0, "oversized capacity {capacity}");
    }

    let exact = Vec::with_capacity(4096);
    let allocation = exact.as_ptr();
    pool.release(exact);
    assert_eq!(pool.idle_count(), 1, "exact capacity remains reusable");
    let reused = pool.acquire();
    assert_eq!(reused.as_ptr(), allocation);
}

#[test]
fn pool_enforces_idle_count_limit_including_zero_max_pool() {
    for max_pool in [0, 1, 3] {
        let pool = ChunkPool::new(max_pool, 64);
        let limit = max_pool.max(1);
        assert_eq!(pool.capacity(), limit);
        for returned in 1..=limit + 1 {
            pool.release(Vec::with_capacity(64));
            assert_eq!(pool.idle_count(), returned.min(limit));
        }
    }
}

#[test]
fn pool_acquire_reserves_from_length_after_small_return() {
    let pool = ChunkPool::new(1, 4096);
    for capacity in [0, 3072, 4095] {
        pool.release(Vec::with_capacity(capacity));
        assert_eq!(pool.idle_count(), 1);
        let acquired = pool.acquire();
        assert!(acquired.is_empty());
        assert_eq!(acquired.capacity(), 4096);
    }
}

#[test]
fn pool_bounds_sum_of_idle_capacities() {
    let chunk_size = 4096;
    let pool = ChunkPool::new(3, chunk_size);
    for capacity in [
        chunk_size + 1,
        chunk_size / 2,
        chunk_size * 8,
        chunk_size,
        chunk_size - 1,
        chunk_size,
    ] {
        pool.release(Vec::with_capacity(capacity));
    }
    assert_eq!(pool.idle_count(), pool.capacity());

    let mut retained = 0;
    while let Some(buf) = pool.queue.pop() {
        assert!(buf.is_empty());
        assert!(buf.capacity() <= chunk_size);
        retained += buf.capacity();
    }
    assert!(retained <= pool.capacity() * chunk_size);
}

#[test]
fn pooled_chunk_recycles_only_after_final_clone_or_slice_drop() {
    let pool = Arc::new(ChunkPool::new(2, 64));
    let mut buf = pool.acquire();
    buf.extend_from_slice(b"consumer-owned");
    let allocation = buf.as_ptr();
    let original = Bytes::from_owner(PooledChunk::new(buf, Arc::clone(&pool)));
    let clone = original.clone();
    let slice = clone.slice(9..);
    assert_eq!(pool.idle_count(), 0);
    drop(original);
    assert_eq!(pool.idle_count(), 0);
    drop(clone);
    assert_eq!(pool.idle_count(), 0);
    assert_eq!(slice.as_ref(), b"owned");

    std::thread::spawn(move || drop(slice)).join().unwrap();
    assert_eq!(pool.idle_count(), 1);
    let recycled = pool.acquire();
    assert_eq!(recycled.as_ptr(), allocation);
    assert!(recycled.is_empty());
}

#[test]
fn large_write_remains_one_chunk_and_is_not_retained_after_last_clone() {
    let chunk_size = StreamingWriter::CHUNK_TARGET + StreamingWriter::BUF_HEADROOM;
    let pool = Arc::new(ChunkPool::new(4, chunk_size));
    let (tx, mut rx) = channel(8);
    let mut writer = StreamingWriter::new_pooled(tx, Arc::clone(&pool));
    assert_eq!(writer.buf.capacity(), chunk_size);
    let large = "x".repeat(chunk_size * 3);
    writer.write("prefix").unwrap();
    writer.write(&large).unwrap();
    writer.end().unwrap();
    assert_eq!(rx.len(), 1, "the coalescing target is not a byte cap");

    let original = rx.try_recv().unwrap();
    assert_eq!(&original[..6], b"prefix");
    assert_eq!(&original[6..], large.as_bytes());
    assert!(original.len() > chunk_size);
    let clone = original.clone();
    drop(original);
    assert_eq!(pool.idle_count(), 0);
    assert_eq!(&clone[6..], large.as_bytes());
    drop(clone);
    assert_eq!(pool.idle_count(), 0, "grown allocation must be dropped");

    writer.write("small").unwrap();
    writer.end().unwrap();
    let small = rx.try_recv().unwrap();
    assert_eq!(small.as_ref(), b"small");
    drop(small);
    assert_eq!(pool.idle_count(), 1, "normal chunks still recycle");
    drop(writer);
    assert_eq!(pool.idle_count(), 2, "writer's active buffer also returns");
    assert!(rx.try_recv().is_err());
}
