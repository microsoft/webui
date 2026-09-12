// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Benchmarks comparing buffered vs streaming render paths.
//!
//! Render benchmark groups against the real contact-book-manager protocol
//! at three contact scales (10/100/1000):
//!
//! ## `writer_paths` — total render throughput
//!
//! Compares four writer paths head-to-head, measuring **total** render
//! time (producer + consumer drain). All paths produce byte-identical
//! output; the only thing changing is how the bytes are delivered.
//!
//! 1. **String** — baseline. Pre-allocated `String` buffer.
//! 2. **StreamingWriter** — bounded tokio mpsc, default capacity = 4 chunks.
//! 3. **StreamingWriter + RenderOptions inject** — production path:
//!    head/body inject HTML emitted by the handler at the structural
//!    `head_end`/`body_end` signal boundaries. Zero scan cost.
//! 4. **String + post-render inject** — mirrors the legacy
//!    `lr.inject(&buf)` path the streaming work replaces.
//!
//! ## `ttfb` — time-to-first-byte
//!
//! Measures the latency from "render started" to "first chunk available
//! to the consumer." This is the metric streaming was designed to
//! improve. For each scenario, compares:
//!
//! * **buffered_ttfb** — String render: full render time (no chunks
//!   until end).
//! * **streaming_ttfb** — Streaming render: time until first 4 KB
//!   chunk is available on the receiver.
//!
//! ## `transport` / `transport_tiny` — transport-only costs
//!
//! Large raw/attribute writes and repeated small writes use the default
//! four-slot channel with a persistent concurrent consumer. Tiny writes
//! drain on the producer thread to isolate the hot path from scheduling.
//! Both groups compare pooled and unpooled buffers.
//!
//! Run with: `cargo bench -p microsoft-webui --bench streaming_bench`

#![allow(missing_docs)]

use bytes::Bytes;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use serde_json::{json, Value};
use std::hint::black_box;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use webui::streaming::{ChunkPool, StreamingWriter};
use webui::{build, BuildOptions, CssStrategy, Protocol, ResponseWriter, WebUIHandler};
use webui_handler::RenderOptions;

const CONTACT_COUNTS: &[usize] = &[10, 100, 1000];
const MEASUREMENT_TIME: Duration = Duration::from_secs(8);
const SAMPLE_SIZE: usize = 50;

const HEAD_INJECT: &str = r#"<link rel="preload" as="image" href="/img/hero.jpg" fetchpriority="high"><link rel="preload" as="image" href="/img/p1.jpg"><link rel="preload" as="image" href="/img/p2.jpg">"#;
const BODY_INJECT: &str = r#"<script>(function(){var e=new EventSource('/__webui/livereload');e.addEventListener('reload',function(){location.reload()})})();</script>"#;

// ── State generation ──────────────────────────────────────────────────

const FIRST_NAMES: &[&str] = &[
    "Sarah", "Marcus", "Yuki", "Priya", "James", "Amara", "Luis", "Emma", "David", "Fatima",
];
const LAST_NAMES: &[&str] = &[
    "Chen",
    "Johnson",
    "Tanaka",
    "Sharma",
    "O'Brien",
    "Okafor",
    "Ramirez",
    "Lindström",
    "Kim",
    "Al-Hassan",
];
const GROUPS: &[&str] = &["Family", "Work", "Friends", "Other"];

fn generate_contact(idx: usize) -> Value {
    let first = FIRST_NAMES[idx % FIRST_NAMES.len()];
    let last = LAST_NAMES[idx % LAST_NAMES.len()];
    json!({
        "id": (idx + 1).to_string(),
        "firstName": first,
        "lastName": last,
        "email": format!("{}.{}@example.com", first.to_lowercase(), last.to_lowercase()),
        "phone": format!("+1 (555) {:03}-{:04}", (idx * 111) % 1000, (idx * 1234) % 10000),
        "company": "Contoso Ltd",
        "group": GROUPS[idx % GROUPS.len()],
        "favorite": idx.is_multiple_of(3),
        "initials": format!("{}{}", &first[..1], &last[..1]),
        "avatarColor": "#4A90D9",
        "notes": String::new(),
        "address": format!("{} St, Seattle, WA", (idx + 1) * 100),
    })
}

fn build_state(count: usize) -> Value {
    let contacts: Vec<Value> = (0..count).map(generate_contact).collect();
    let recent: Vec<Value> = contacts[count.saturating_sub(5)..].to_vec();
    json!({
        "page": "dashboard",
        "searchQuery": "",
        "activeGroup": "all",
        "groups": GROUPS,
        "totalContacts": count,
        "totalFavorites": 0,
        "totalGroups": GROUPS.len(),
        "contacts": contacts.clone(),
        "filteredContacts": contacts,
        "recentContacts": recent,
        "favoriteContacts": Vec::<Value>::new(),
        "selectedContact": null,
    })
}

fn build_protocol() -> Arc<Protocol> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let app_dir = manifest
        .join("..")
        .join("..")
        .join("examples")
        .join("app")
        .join("contact-book-manager")
        .join("src");
    let document = build(BuildOptions {
        app_dir,
        entry: "index.html".to_string(),
        css: CssStrategy::Style,
        ..BuildOptions::default()
    })
    .expect("failed to build contact-book-manager protocol")
    .protocol;
    Arc::new(Protocol::new(document))
}

// ── Writers ────────────────────────────────────────────────────────────

struct StringWriter {
    buf: String,
}
impl StringWriter {
    fn with_capacity(cap: usize) -> Self {
        Self {
            buf: String::with_capacity(cap),
        }
    }
}
impl ResponseWriter for StringWriter {
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        self.buf.push_str(content);
        Ok(())
    }
    fn end(&mut self) -> webui_handler::Result<()> {
        Ok(())
    }
}

/// Drain a tokio mpsc receiver synchronously, summing bytes received.
/// Uses `blocking_recv`; no async runtime is involved.
fn drain_total(mut rx: mpsc::Receiver<Bytes>) -> usize {
    let mut total = 0;
    while let Some(chunk) = rx.blocking_recv() {
        total += chunk.len();
    }
    total
}

// ── writer_paths group: total render throughput ───────────────────────

fn bench_writers(c: &mut Criterion) {
    let protocol = build_protocol();
    let states: Vec<(usize, Value)> = CONTACT_COUNTS
        .iter()
        .map(|&n| (n, build_state(n)))
        .collect();

    // Measure output size per scenario (used for throughput).
    let sizes: Vec<usize> = states
        .iter()
        .map(|(_, state)| {
            let h = WebUIHandler::new();
            let mut w = StringWriter::with_capacity(512 * 1024);
            h.render(
                &protocol,
                state,
                &RenderOptions::new("index.html", "/"),
                &mut w,
            )
            .unwrap();
            w.buf.len()
        })
        .collect();

    let mut group = c.benchmark_group("writer_paths");
    group.measurement_time(MEASUREMENT_TIME);
    group.sample_size(SAMPLE_SIZE);

    for ((count, state), &output_size) in states.iter().zip(sizes.iter()) {
        group.throughput(Throughput::Bytes(output_size as u64));

        // Path 1: String (baseline).
        group.bench_with_input(
            BenchmarkId::new(format!("string/{count}"), output_size),
            state,
            |b, state| {
                let h = WebUIHandler::new();
                b.iter(|| {
                    let mut w = StringWriter::with_capacity(output_size);
                    h.render(
                        black_box(&protocol),
                        black_box(state),
                        &RenderOptions::new("index.html", "/"),
                        &mut w,
                    )
                    .unwrap();
                    black_box(w.buf.len());
                });
            },
        );

        // Path 2: StreamingWriter (bounded). Drain on the same thread
        // by running the producer first (fills channel up to its
        // capacity, then producer would block) — but with chunks
        // sized to fit in the channel we don't block.
        // To measure honestly without a separate thread, we use a
        // capacity that holds the entire output (~16 chunks for 64 KB).
        group.bench_with_input(
            BenchmarkId::new(format!("streaming/{count}"), output_size),
            state,
            |b, state| {
                let h = WebUIHandler::new();
                let cap = (output_size / StreamingWriter::CHUNK_TARGET) + 4;
                b.iter(|| {
                    let (tx, rx) = mpsc::channel::<Bytes>(cap);
                    let mut w = StreamingWriter::new(tx);
                    h.render(
                        black_box(&protocol),
                        black_box(state),
                        &RenderOptions::new("index.html", "/"),
                        &mut w,
                    )
                    .unwrap();
                    drop(w);
                    black_box(drain_total(rx));
                });
            },
        );

        // Path 3: Streaming + RenderOptions inject (production path).
        // The contact-book template is Shadow DOM (no <head>/<body>),
        // so the head_end/body_end signals never fire; the inject
        // strings are configured but unused. Cost = essentially the
        // same as path 2 (streaming alone).
        group.bench_with_input(
            BenchmarkId::new(format!("streaming+inject(opts)/{count}"), output_size),
            state,
            |b, state| {
                let h = WebUIHandler::new();
                let cap = (output_size / StreamingWriter::CHUNK_TARGET) + 4;
                b.iter(|| {
                    let (tx, rx) = mpsc::channel::<Bytes>(cap);
                    let mut w = StreamingWriter::new(tx);
                    let opts = RenderOptions::new("index.html", "/")
                        .with_head_inject(HEAD_INJECT)
                        .with_body_inject(BODY_INJECT);
                    h.render(black_box(&protocol), black_box(state), &opts, &mut w)
                        .unwrap();
                    drop(w);
                    black_box(drain_total(rx));
                });
            },
        );

        // Path 4: String + post-render injection (mirrors the OLD
        // livereload path the streaming work replaces).
        group.bench_with_input(
            BenchmarkId::new(format!("string+postinject/{count}"), output_size),
            state,
            |b, state| {
                let h = WebUIHandler::new();
                b.iter(|| {
                    let mut w = StringWriter::with_capacity(output_size);
                    h.render(
                        black_box(&protocol),
                        black_box(state),
                        &RenderOptions::new("index.html", "/"),
                        &mut w,
                    )
                    .unwrap();
                    let merged = post_inject(&w.buf, BODY_INJECT);
                    black_box(merged.len());
                });
            },
        );
    }
    group.finish();
}

/// Mirror of the legacy livereload injection: case-insensitive
/// `</body>` byte-window scan, then concatenate into a new String.
fn post_inject(html: &str, script: &str) -> String {
    if let Some(idx) = html
        .as_bytes()
        .windows(7)
        .position(|w| w.eq_ignore_ascii_case(b"</body>"))
    {
        let mut out = String::with_capacity(html.len() + script.len() + 2);
        out.push_str(&html[..idx]);
        out.push_str(script);
        out.push_str(&html[idx..]);
        out
    } else {
        let mut out = String::with_capacity(html.len() + script.len());
        out.push_str(html);
        out.push_str(script);
        out
    }
}

// ── ttfb group: time-to-first-byte (the streaming claim) ──────────────

/// Spawn the render on a dedicated thread (mirroring the production
/// `spawn_blocking` shape) and measure the time from "spawn" to "first
/// chunk available on the receiver." This is what the user sees as
/// "time to first byte" minus network latency.
///
/// Note: we deliberately drop the receiver after the first chunk to
/// measure latency, which causes the producer to error out with
/// `ClientDisconnected` on its next flush — that's the *correct*
/// production behaviour (cancel the render). We swallow that error
/// here because it's expected.
fn streaming_ttfb(protocol: &Arc<Protocol>, state: &Value) -> Duration {
    let (tx, mut rx) = mpsc::channel::<Bytes>(StreamingWriter::DEFAULT_CHANNEL_CAPACITY);
    let proto = protocol.clone();
    let st = state.clone();
    let start = Instant::now();
    std::thread::spawn(move || {
        let h = WebUIHandler::new();
        let mut w = StreamingWriter::new(tx);
        // Both calls may legitimately return Err(ClientDisconnected)
        // when the bench drops the receiver after the first chunk —
        // that's the production-correct cancellation path.
        if h.render(&proto, &st, &RenderOptions::new("index.html", "/"), &mut w)
            .is_err()
        {
            let _ = ResponseWriter::end(&mut w);
        }
    });
    // Block until the first chunk arrives.
    let _ = rx.blocking_recv();
    start.elapsed()
}

/// Buffered baseline: the receiver only sees bytes when the entire
/// render has completed and the result is handed off. This is what
/// `pnpm start:server` did before streaming.
fn buffered_ttfb(protocol: &Protocol, state: &Value) -> Duration {
    let h = WebUIHandler::new();
    let cap = 64 * 1024;
    let start = Instant::now();
    let mut w = StringWriter::with_capacity(cap);
    h.render(
        protocol,
        state,
        &RenderOptions::new("index.html", "/"),
        &mut w,
    )
    .unwrap();
    // "First byte" is when the response is complete in the buffered
    // model — there's nothing to send before that.
    start.elapsed()
}

fn bench_ttfb(c: &mut Criterion) {
    let protocol = build_protocol();
    let states: Vec<(usize, Value)> = CONTACT_COUNTS
        .iter()
        .map(|&n| (n, build_state(n)))
        .collect();

    let mut group = c.benchmark_group("ttfb");
    group.measurement_time(MEASUREMENT_TIME);
    group.sample_size(SAMPLE_SIZE);

    for (count, state) in &states {
        group.bench_with_input(BenchmarkId::new("buffered", count), state, |b, state| {
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    total += buffered_ttfb(&protocol, state);
                }
                total
            });
        });

        group.bench_with_input(BenchmarkId::new("streaming", count), state, |b, state| {
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    total += streaming_ttfb(&protocol, state);
                }
                total
            });
        });
    }
    group.finish();
}

// ── transport group: bounded channel and chunk-buffer costs ──────────

#[derive(Clone, Copy)]
enum TransportWrites<'a> {
    Raw(&'a str),
    Attribute(&'a str),
    Small,
}

impl TransportWrites<'_> {
    fn output_size(self) -> usize {
        match self {
            Self::Raw(content) => content.len(),
            Self::Attribute(value) => value.len() + " data-value=\"\"".len(),
            Self::Small => 1024 * 16,
        }
    }

    fn write(self, writer: &mut StreamingWriter) {
        match self {
            Self::Raw(content) => writer.write(black_box(content)).unwrap(),
            Self::Attribute(value) => writer
                .write_attribute(black_box("data-value"), black_box(value))
                .unwrap(),
            Self::Small => {
                for _ in 0..1024 {
                    writer.write(black_box("0123456789abcdef")).unwrap();
                }
            }
        }
        writer.end().unwrap();
    }
}

fn transport_writer(tx: mpsc::Sender<Bytes>, pool: &Option<Arc<ChunkPool>>) -> StreamingWriter {
    match pool {
        Some(pool) => StreamingWriter::new_pooled(tx, Arc::clone(pool)),
        None => StreamingWriter::new(tx),
    }
}

/// The persistent consumer drains the production four-slot channel and
/// acknowledges each response. Thread creation is outside the timing window;
/// channel backpressure, final consumption, and acknowledgement are measured.
fn bench_transport(c: &mut Criterion) {
    let large = "x".repeat(1024 * 1024);
    let cases = [
        ("raw_1m", TransportWrites::Raw(&large)),
        ("attribute_1m", TransportWrites::Attribute(&large)),
        ("small_16k", TransportWrites::Small),
    ];
    let mut group = c.benchmark_group("transport");
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(1));
    group.sample_size(20);

    for (name, writes) in cases {
        group.throughput(Throughput::Bytes(writes.output_size() as u64));
        for pooled in [false, true] {
            let mode = if pooled { "pooled" } else { "unpooled" };
            group.bench_function(BenchmarkId::new(name, mode), |b| {
                let pool =
                    pooled.then(|| Arc::new(ChunkPool::new(16, StreamingWriter::CHUNK_TARGET)));
                let (tx, mut rx) =
                    mpsc::channel::<Bytes>(StreamingWriter::DEFAULT_CHANNEL_CAPACITY);
                let (done_tx, done_rx) = std::sync::mpsc::sync_channel(1);
                let output_size = writes.output_size();
                let consumer = std::thread::spawn(move || {
                    let mut received = 0;
                    while let Some(chunk) = rx.blocking_recv() {
                        received += black_box(chunk.len());
                        drop(chunk);
                        if received == output_size {
                            done_tx.send(()).unwrap();
                            received = 0;
                        }
                    }
                    assert_eq!(received, 0);
                });
                b.iter(|| {
                    let mut writer = transport_writer(tx.clone(), &pool);
                    writes.write(&mut writer);
                    drop(writer);
                    done_rx.recv().unwrap();
                });
                drop(tx);
                consumer.join().unwrap();
            });
        }
    }
    group.finish();
}

/// Isolate the tiny-write fast path without cross-thread scheduling noise.
fn bench_transport_tiny(c: &mut Criterion) {
    let mut group = c.benchmark_group("transport_tiny");
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(1));
    group.sample_size(20);
    for attributes in [false, true] {
        let name = if attributes { "attributes" } else { "raw" };
        for pooled in [false, true] {
            let mode = if pooled { "pooled" } else { "unpooled" };
            let pool = pooled.then(|| Arc::new(ChunkPool::new(4, StreamingWriter::CHUNK_TARGET)));
            group.bench_function(BenchmarkId::new(name, mode), |b| {
                b.iter(|| {
                    let (tx, rx) = mpsc::channel(StreamingWriter::DEFAULT_CHANNEL_CAPACITY);
                    let mut writer = transport_writer(tx, &pool);
                    for _ in 0..32 {
                        if attributes {
                            writer
                                .write_attribute(black_box("id"), black_box("value"))
                                .unwrap();
                            writer.write_boolean_attribute(black_box("hidden")).unwrap();
                        } else {
                            writer.write(black_box("12345678")).unwrap();
                        }
                    }
                    writer.end().unwrap();
                    drop(writer);
                    black_box(drain_total(rx));
                });
            });
        }
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_writers,
    bench_ttfb,
    bench_transport,
    bench_transport_tiny
);
criterion_main!(benches);
