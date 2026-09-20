// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Benchmarks comparing buffered vs streaming render paths.
//!
//! Two benchmark groups against the real contact-book-manager protocol
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
use webui::streaming::StreamingWriter;
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
/// Uses `try_recv` in a tight loop because the producer thread fills
/// the channel before the bench iteration ends; no async runtime is
/// involved in the measurement window.
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

criterion_group!(benches, bench_writers, bench_ttfb);
criterion_main!(benches);
