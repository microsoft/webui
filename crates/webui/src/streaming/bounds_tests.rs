// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

use super::*;
use tokio::sync::mpsc::{channel, Receiver};

fn assert_chunks(mut rx: Receiver<Bytes>, expected: &str, maximum: usize) -> bool {
    let mut output = Vec::with_capacity(expected.len());
    let mut split_utf8 = false;
    while let Ok(chunk) = rx.try_recv() {
        assert!(!chunk.is_empty());
        assert!(chunk.len() <= maximum, "{} > {maximum}", chunk.len());
        split_utf8 |= std::str::from_utf8(&chunk).is_err();
        output.extend_from_slice(&chunk);
    }
    assert_eq!(output, expected.as_bytes());
    split_utf8
}

#[test]
fn large_raw_writes_have_a_hard_byte_maximum() {
    let content = "€𠜎é".repeat(4097);
    for maximum in [64, 127, StreamingWriter::CHUNK_TARGET] {
        let expected = format!("{}{}", "a".repeat(maximum - 1), content);
        for pooled in [false, true] {
            let (tx, rx) = channel(expected.len() / maximum + 2);
            let mut writer = if pooled {
                StreamingWriter::new_pooled(tx, Arc::new(ChunkPool::new(4, maximum)))
            } else {
                StreamingWriter::new(tx)
            }
            .with_chunk_size(maximum);
            writer.write(&expected[..maximum - 1]).unwrap();
            writer.write(&content).unwrap();
            writer.end().unwrap();
            assert!(writer.buf.capacity() <= maximum);
            drop(writer);
            assert!(assert_chunks(rx, &expected, maximum));
        }
    }
}

#[test]
fn quoted_and_boolean_attributes_preserve_bytes_at_every_boundary() {
    let long_name = "界".repeat(83);
    let long_value = "€𠜎é&amp;".repeat(97);
    for maximum in [64, 127] {
        for prefix_len in 0..maximum {
            let (tx, rx) = channel(128);
            let mut writer = StreamingWriter::new(tx).with_chunk_size(maximum);
            let mut expected = "p".repeat(prefix_len);
            writer.write(&expected).unwrap();
            for (name, value) in [
                ("id", "value"),
                (long_name.as_str(), long_value.as_str()),
                ("", ""),
            ] {
                writer.write_attribute(name, value).unwrap();
                webui_handler::append_attribute_to_string(&mut expected, name, value);
            }
            for name in ["hidden", long_name.as_str(), ""] {
                writer.write_boolean_attribute(name).unwrap();
                webui_handler::append_boolean_attribute_to_string(&mut expected, name);
            }
            writer.end().unwrap();
            assert!(writer.buf.capacity() <= maximum);
            drop(writer);
            assert_chunks(rx, &expected, maximum);
        }
    }
}

#[test]
fn explicit_flush_preserves_record_boundaries_after_chunk_splitting() {
    let (tx, mut rx) = channel(8);
    let mut writer = StreamingWriter::new(tx).with_chunk_size(64);
    let record = "€".repeat(45);
    writer.write(&record).unwrap();
    assert_eq!(
        rx.len(),
        2,
        "two full chunks, partial suffix still buffered"
    );
    writer.flush().unwrap();
    assert_eq!(rx.len(), 3);
    writer.flush().unwrap();
    assert_eq!(rx.len(), 3, "empty semantic flush emits no chunk");

    let mut first_record = Vec::new();
    while let Ok(chunk) = rx.try_recv() {
        assert!(chunk.len() <= 64);
        first_record.extend_from_slice(&chunk);
    }
    assert_eq!(first_record, record.as_bytes());
    writer.write("next record").unwrap();
    assert!(rx.try_recv().is_err());
    writer.end().unwrap();
    drop(writer);
    assert_chunks(rx, "next record", 64);
}

#[derive(Clone, Copy, Debug)]
enum LargeWrite {
    Raw,
    AttributeName,
    AttributeValue,
    Boolean,
}

impl LargeWrite {
    fn send(self, writer: &mut StreamingWriter, content: &str) -> Result<()> {
        match self {
            Self::Raw => writer.write(content),
            Self::AttributeName => writer.write_attribute(content, "v"),
            Self::AttributeValue => writer.write_attribute("key", content),
            Self::Boolean => writer.write_boolean_attribute(content),
        }
    }
}

const LARGE_WRITES: [LargeWrite; 4] = [
    LargeWrite::Raw,
    LargeWrite::AttributeName,
    LargeWrite::AttributeValue,
    LargeWrite::Boolean,
];

#[test]
fn disconnect_interrupts_each_kind_of_multi_chunk_write() {
    for kind in LARGE_WRITES {
        let (tx, mut rx) = channel::<Bytes>(1);
        let producer = std::thread::spawn(move || {
            let mut writer = StreamingWriter::new(tx).with_chunk_size(64);
            let result = kind.send(&mut writer, &"x".repeat(64 * 100));
            assert!(matches!(result, Err(HandlerError::ClientDisconnected)));
            assert!(writer.is_terminated());
            assert!(writer.buf.is_empty());
            assert!(matches!(
                writer.flush(),
                Err(HandlerError::ClientDisconnected)
            ));
            for subsequent in LARGE_WRITES {
                assert!(matches!(
                    subsequent.send(&mut writer, "ignored"),
                    Err(HandlerError::ClientDisconnected)
                ));
            }
            writer.end().unwrap();
        });
        let first = rx.blocking_recv().unwrap();
        // Drop before asserting so a baseline failure cannot strand the sender.
        drop(rx);
        producer.join().unwrap();
        assert_eq!(first.len(), 64);
    }
}

#[test]
fn timeout_interrupts_each_kind_of_multi_chunk_write_without_runtime() {
    for kind in LARGE_WRITES {
        let (tx, mut rx) = channel(1);
        let mut writer = StreamingWriter::new(tx)
            .with_chunk_size(64)
            .with_flush_timeout(Duration::from_millis(10));
        let started = Instant::now();
        let result = kind.send(&mut writer, &"x".repeat(64 * 100));
        assert!(
            matches!(result, Err(HandlerError::StreamTimeout)),
            "{kind:?}"
        );
        assert!(started.elapsed() >= Duration::from_millis(10));
        assert!(writer.is_terminated());
        assert!(writer.buf.is_empty());
        assert!(matches!(writer.flush(), Err(HandlerError::StreamTimeout)));
        for subsequent in LARGE_WRITES {
            assert!(matches!(
                subsequent.send(&mut writer, "ignored"),
                Err(HandlerError::StreamTimeout)
            ));
        }
        assert_eq!(rx.try_recv().unwrap().len(), 64);
        assert!(rx.try_recv().is_err(), "unsent suffix must not escape");
        writer.end().unwrap();
    }
}

#[test]
fn timeout_interrupts_multi_chunk_write_on_tokio_blocking_worker() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_time()
        .build()
        .unwrap();
    runtime.block_on(async {
        let (tx, mut rx) = channel(1);
        let producer = tokio::task::spawn_blocking(move || {
            let mut writer = StreamingWriter::new(tx)
                .with_chunk_size(64)
                .with_flush_timeout(Duration::from_millis(10));
            let result = writer.write_attribute("key", &"x".repeat(1024));
            assert!(writer.is_terminated());
            result
        });
        assert!(matches!(
            producer.await.unwrap(),
            Err(HandlerError::StreamTimeout)
        ));
        assert_eq!(rx.try_recv().unwrap().len(), 64);
        assert!(rx.try_recv().is_err());
    });
}

#[test]
fn pool_rejects_oversized_returns_instead_of_retaining_or_shrinking_them() {
    let pool = ChunkPool::new(4, 4096);
    for capacity in [4097, 1024 * 1024] {
        let mut oversized = Vec::with_capacity(capacity);
        oversized.extend_from_slice(b"leftover");
        pool.release(oversized);
        assert_eq!(pool.idle_count(), 0, "oversized capacity {capacity}");
    }
    pool.release(Vec::with_capacity(4096));
    assert_eq!(pool.idle_count(), 1, "exact limit is reusable");
}

#[test]
fn pool_reserves_from_length_not_capacity() {
    let pool = ChunkPool::new(4, 4096);
    pool.release(Vec::with_capacity(3072));
    let acquired = pool.acquire();
    assert!(acquired.is_empty());
    assert_eq!(acquired.capacity(), 4096);
}

#[test]
fn pooled_writer_sizes_initial_and_replacement_buffers() {
    let pool = Arc::new(ChunkPool::new(4, 3072));
    let (tx, rx) = channel(4);
    let mut writer = StreamingWriter::new_pooled(tx, Arc::clone(&pool));
    assert_eq!(writer.buf.capacity(), StreamingWriter::CHUNK_TARGET);
    writer.write(&"x".repeat(4096)).unwrap();
    assert_eq!(writer.buf.capacity(), StreamingWriter::CHUNK_TARGET);
    drop(writer);
    drop(rx);
    assert_eq!(pool.idle_count(), 0, "grown buffers exceed this pool's cap");

    let (tx, rx) = channel(4);
    let mut writer = StreamingWriter::new_pooled(tx, Arc::clone(&pool)).with_chunk_size(5000);
    writer.write(&"x".repeat(5000)).unwrap();
    assert_eq!(writer.buf.capacity(), 5000);
    drop(writer);
    drop(rx);
    assert_eq!(pool.idle_count(), 0);
}

#[test]
fn pooled_chunk_slices_return_capacity_only_after_last_consumer_drop() {
    let pool = Arc::new(ChunkPool::new(4, 64));
    let (tx, mut rx) = channel(4);
    let mut writer = StreamingWriter::new_pooled(tx, Arc::clone(&pool)).with_chunk_size(64);
    writer.write(&"x".repeat(64)).unwrap();
    let original = rx.try_recv().unwrap();
    let clone = original.clone();
    let slice = clone.slice(3..19);
    assert_eq!(pool.idle_count(), 0);
    drop(original);
    drop(clone);
    assert_eq!(pool.idle_count(), 0);
    assert_eq!(slice.as_ref(), b"xxxxxxxxxxxxxxxx");
    std::thread::spawn(move || drop(slice)).join().unwrap();
    assert_eq!(pool.idle_count(), 1);
    drop(writer);
    assert_eq!(pool.idle_count(), 2);
}

#[test]
fn oversized_chunk_is_dropped_after_last_consumer_without_pool_retention() {
    let pool = Arc::new(ChunkPool::new(4, 64));
    let mut huge = Vec::with_capacity(1024 * 1024);
    huge.extend_from_slice(b"still readable");
    let payload = Bytes::from_owner(PooledChunk::new(huge, Arc::clone(&pool)));
    let clone = payload.clone();
    drop(payload);
    assert_eq!(clone.as_ref(), b"still readable");
    assert_eq!(pool.idle_count(), 0);
    drop(clone);
    assert_eq!(pool.idle_count(), 0);
}

#[test]
fn slow_consumer_keeps_default_channel_and_idle_pool_byte_bounded() {
    let maximum = StreamingWriter::CHUNK_TARGET;
    let pool = Arc::new(ChunkPool::new(4, maximum));
    let producer_pool = Arc::clone(&pool);
    let expected = "€𠜎é".repeat(16 * 1024);
    let content = expected.clone();
    let (tx, mut rx) = channel::<Bytes>(StreamingWriter::DEFAULT_CHANNEL_CAPACITY);
    let producer = std::thread::spawn(move || {
        let mut writer = StreamingWriter::new_pooled(tx, producer_pool);
        writer.write_attribute("data-value", &content).unwrap();
        writer.end().unwrap();
        assert!(writer.buf.capacity() <= maximum);
    });
    let mut received = Vec::with_capacity(expected.len() + 14);
    while let Some(chunk) = rx.blocking_recv() {
        assert!(chunk.len() <= maximum);
        received.extend_from_slice(&chunk);
        std::thread::sleep(Duration::from_micros(100));
    }
    producer.join().unwrap();
    assert_eq!(received, format!(" data-value=\"{expected}\"").as_bytes());
    assert_eq!(pool.idle_count(), pool.capacity());
    let mut retained = 0;
    while let Some(buf) = pool.queue.pop() {
        assert!(buf.is_empty());
        assert!(buf.capacity() <= maximum);
        retained += buf.capacity();
    }
    assert!(retained <= pool.capacity() * maximum);
}
