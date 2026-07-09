// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Memory + CPU benchmark for the streaming render paths.
//!
//! Measures **per-render resource usage** — not just wall-clock time —
//! across the five writer paths exercised by `crates/webui/benches/
//! streaming_bench.rs`:
//!
//! 1. `string`                            — pre-allocated `String` buffer.
//! 2. `streaming`                         — `StreamingWriter` alone.
//! 3. `streaming+inject(opts)`            — production composition with
//!    `RenderOptions::with_head_inject` / `with_body_inject` (handler
//!    emits at the parser-synthesized `head_end` / `body_end` signals).
//! 4. `string+postinject`                 — legacy `lr.inject(&buf)` reference.
//! 5. `streaming+inject(opts) POOLED`     — production path with shared
//!    `ChunkPool` for chunk-buffer recycling.
//!
//! For each path × scale (10 / 100 / 1000 contacts) it reports allocations,
//! bytes allocated, CPU user/system time and the process peak-RSS high-water
//! mark. The measurement primitives (allocation counter, CPU-time + peak-RSS
//! reader, baseline snapshots and result table) come from the shared
//! [`webui_bench_support`] dev crate, so this example only supplies the workload.
//!
//! Unlike criterion (which only reports wall-clock), this gives a direct
//! allocator-level view useful for verifying that the streaming writer's "zero
//! per-write allocation" claim actually holds in the production path.
//!
//! Usage:
//!
//! ```sh
//! cargo run --release --example streaming_resource_bench -p microsoft-webui
//! cargo run --release --example streaming_resource_bench -p microsoft-webui -- --save main
//! cargo run --release --example streaming_resource_bench -p microsoft-webui -- --compare main
//! ```

#![allow(missing_docs)]

use bytes::Bytes;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use webui::streaming::{ChunkPool, StreamingWriter};
use webui::{build, BuildOptions, CssStrategy, ResponseWriter, WebUIHandler};
use webui_bench_support::report::{format_bytes, Align, Table};
use webui_bench_support::{baseline, measure, BaselineRow, CountingAllocator, Measurement, Metric};
use webui_handler::RenderOptions;
use webui_protocol::WebUIProtocol;

// The allocation counter, CPU-time + peak-RSS reader, baseline snapshots and
// result table all live in the shared `webui-bench-support` dev crate; this
// example only installs the counting allocator and supplies the workload.
#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator::new();

/// Baseline row schema; bump on any change to [`Row`]'s fields or metrics.
const SCHEMA: u32 = 2;
/// Baseline file kind — `target/bench-baselines/resource-<name>.json`.
const KIND: &str = "resource";
/// Fixed iteration count per row (these paths are cheap and uniform in cost).
const ITERS_PER_SCALE: usize = 2_000;

// ── State + protocol setup ────────────────────────────────────────────

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

fn build_protocol() -> WebUIProtocol {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let app_dir = manifest
        .join("..")
        .join("..")
        .join("examples")
        .join("app")
        .join("contact-book-manager")
        .join("src");
    build(BuildOptions {
        app_dir,
        entry: "index.html".to_string(),
        css: CssStrategy::Style,
        ..BuildOptions::default()
    })
    .expect("failed to build contact-book-manager protocol")
    .protocol
}

const HEAD_INJECT: &str = r#"<link rel="preload" as="image" href="/img/hero.jpg" fetchpriority="high"><link rel="preload" as="image" href="/img/p1.jpg"><link rel="preload" as="image" href="/img/p2.jpg">"#;
const BODY_INJECT: &str = r#"<script>(function(){var e=new EventSource('/__webui/livereload');e.addEventListener('reload',function(){location.reload()})})();</script>"#;

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

fn drain_total(mut rx: mpsc::Receiver<Bytes>) -> usize {
    let mut total = 0;
    while let Some(chunk) = rx.blocking_recv() {
        total += chunk.len();
    }
    total
}

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

// ── Per-path drivers ──────────────────────────────────────────────────

fn run_string(protocol: &WebUIProtocol, state: &Value, output_size: usize) -> usize {
    let h = WebUIHandler::new();
    let mut w = StringWriter::with_capacity(output_size);
    h.handle(
        protocol,
        state,
        &RenderOptions::new("index.html", "/"),
        &mut w,
    )
    .expect("render");
    w.buf.len()
}

fn run_streaming(protocol: &WebUIProtocol, state: &Value, output_size: usize) -> usize {
    let h = WebUIHandler::new();
    let cap = (output_size / StreamingWriter::CHUNK_TARGET) + 4;
    let (tx, rx) = mpsc::channel::<Bytes>(cap);
    let mut w = StreamingWriter::new(tx);
    h.handle(
        protocol,
        state,
        &RenderOptions::new("index.html", "/"),
        &mut w,
    )
    .expect("render");
    ResponseWriter::end(&mut w).expect("end");
    drop(w);
    drain_total(rx)
}

/// Streaming with `RenderOptions::with_head_inject` /
/// `with_body_inject`. Note: the contact-book template is a Shadow
/// DOM template with no `<head>`/`<body>` tags, so `head_end` /
/// `body_end` signals never fire and the inject strings are NOT
/// emitted. This row therefore measures "inject configured but never
/// triggered" — which on the new signal-based path costs **nothing**
/// (just two `Option<String>` fields on the context). The legacy
/// byte-scanner approach had to scan every output byte looking for
/// never-present markers, costing ~14 µs of pure overhead.
fn run_streaming_with_inject(protocol: &WebUIProtocol, state: &Value, output_size: usize) -> usize {
    let h = WebUIHandler::new();
    let cap = (output_size / StreamingWriter::CHUNK_TARGET) + 4;
    let (tx, rx) = mpsc::channel::<Bytes>(cap);
    let mut w = StreamingWriter::new(tx);
    let opts = RenderOptions::new("index.html", "/")
        .with_head_inject(HEAD_INJECT)
        .with_body_inject(BODY_INJECT);
    h.handle(protocol, state, &opts, &mut w).expect("render");
    ResponseWriter::end(&mut w).expect("end");
    drop(w);
    drain_total(rx)
}

/// Production composition with the lock-free shared chunk pool +
/// signal-based inject. `pool` is shared across all calls (lives for
/// the whole bench run) to mirror the actual server's startup-time
/// pool.
fn run_streaming_pooled_with_inject(
    protocol: &WebUIProtocol,
    state: &Value,
    output_size: usize,
    pool: &Arc<ChunkPool>,
) -> usize {
    let h = WebUIHandler::new();
    let cap = (output_size / StreamingWriter::CHUNK_TARGET) + 4;
    let (tx, rx) = mpsc::channel::<Bytes>(cap);
    let mut w = StreamingWriter::new_pooled(tx, Arc::clone(pool));
    let opts = RenderOptions::new("index.html", "/")
        .with_head_inject(HEAD_INJECT)
        .with_body_inject(BODY_INJECT);
    h.handle(protocol, state, &opts, &mut w).expect("render");
    ResponseWriter::end(&mut w).expect("end");
    drop(w);
    // Drain consumes the Bytes — drops PooledChunk owners — releases
    // chunk Vec back to the pool. This is exactly the actix lifecycle.
    drain_total(rx)
}

fn run_string_postinject(protocol: &WebUIProtocol, state: &Value, output_size: usize) -> usize {
    let h = WebUIHandler::new();
    let mut w = StringWriter::with_capacity(output_size);
    h.handle(
        protocol,
        state,
        &RenderOptions::new("index.html", "/"),
        &mut w,
    )
    .expect("render");
    let merged = post_inject(&w.buf, BODY_INJECT);
    merged.len()
}

fn warmup_output_size(protocol: &WebUIProtocol, state: &Value) -> usize {
    let h = WebUIHandler::new();
    let mut w = StringWriter::with_capacity(128 * 1024);
    h.handle(
        protocol,
        state,
        &RenderOptions::new("index.html", "/"),
        &mut w,
    )
    .expect("warmup");
    w.buf.len()
}

// ── Result row (measurement → report + baseline) ──────────────────────

/// One `path/scale` result: the shared [`Measurement`] projected to per-run
/// metrics, in a shape that both prints and round-trips through a baseline.
#[derive(serde::Serialize, serde::Deserialize)]
struct Row {
    label: String,
    output_bytes: usize,
    iters: usize,
    allocs_per_run: f64,
    bytes_per_run: f64,
    user_us: f64,
    sys_us: f64,
    wall_us: f64,
    rss_high_water_bytes: i64,
}

impl Row {
    fn from_measurement(label: &str, output_bytes: usize, m: Measurement) -> Self {
        let pi = m.per_iter();
        Self {
            label: label.to_string(),
            output_bytes,
            iters: m.iters,
            allocs_per_run: pi.allocs,
            bytes_per_run: pi.bytes,
            user_us: pi.user_us,
            sys_us: pi.sys_us,
            wall_us: pi.wall_us,
            rss_high_water_bytes: pi.max_rss_bytes,
        }
    }
}

impl BaselineRow for Row {
    fn key(&self) -> String {
        self.label.clone()
    }

    /// Memory is the point of this bench (exact allocs/bytes); CPU + latency are
    /// secondary. Peak RSS is a process-wide, cumulative high-water mark, so it
    /// is intentionally **not** gated (too noisy to threshold).
    fn metrics(&self) -> Vec<Metric> {
        vec![
            Metric::lower_better("allocs", self.allocs_per_run),
            Metric::lower_better("bytes", self.bytes_per_run),
            Metric::lower_better("user µs", self.user_us),
            Metric::lower_better("wall µs", self.wall_us),
        ]
    }
}

fn format_rss(bytes: i64) -> String {
    if bytes < 0 {
        return "n/a".to_string();
    }
    if bytes < 1024 * 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{:.2} MiB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn render_table(rows: &[Row]) {
    println!();
    let mut table = Table::new([
        "path/scale (output bytes)",
        "iters",
        "allocs/run",
        "bytes/run",
        "wall µs",
        "user µs/run",
        "sys µs/run",
        "process RSS",
    ])
    .aligns([Align::Left]);
    for r in rows {
        table.row([
            format!("{} ({}B)", r.label, r.output_bytes),
            r.iters.to_string(),
            format!("{:.2}", r.allocs_per_run),
            format_bytes(r.bytes_per_run),
            format!("{:.2}", r.wall_us),
            format!("{:.2}", r.user_us),
            format!("{:.2}", r.sys_us),
            format_rss(r.rss_high_water_bytes),
        ]);
    }
    table.print();
}

// ── CLI parsing ───────────────────────────────────────────────────────

enum Mode {
    Print,
    Save(String),
    Compare(String),
}

fn parse_args() -> Mode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--save" => {
                return iter.next().map(Mode::Save).unwrap_or_else(|| {
                    eprintln!("--save requires a baseline name");
                    std::process::exit(2);
                });
            }
            "--compare" => {
                return iter.next().map(Mode::Compare).unwrap_or_else(|| {
                    eprintln!("--compare requires a baseline name");
                    std::process::exit(2);
                });
            }
            "--help" | "-h" => {
                println!(
                    "Usage: streaming_resource_bench [--save NAME] [--compare NAME]\n\n\
                     With no args: prints the table.\n\
                     --save NAME: write current results to target/bench-baselines/resource-NAME.json\n\
                     --compare NAME: print results AND a Δ%-table vs the saved baseline"
                );
                std::process::exit(0);
            }
            other => {
                eprintln!("unknown arg: {other}");
                std::process::exit(2);
            }
        }
    }
    Mode::Print
}

// ── Main ──────────────────────────────────────────────────────────────

fn main() {
    let mode = parse_args();
    let scales = [10usize, 100, 1000];

    println!("WebUI streaming resource benchmark");
    println!("==================================");
    println!(
        "Build: {} | iterations per row: {}",
        if cfg!(debug_assertions) {
            "DEBUG (numbers will be misleading; rebuild with --release)"
        } else {
            "release"
        },
        ITERS_PER_SCALE
    );
    println!(
        "RSS column = process-wide high-water mark observed at end of phase \
         (cumulative across all phases, only meaningful as a peak)."
    );

    let protocol = build_protocol();

    // One pool shared across the whole bench — this is exactly how the
    // production server uses it (constructed at startup, lives forever).
    let pool = Arc::new(ChunkPool::new(256, StreamingWriter::CHUNK_TARGET + 1024));

    let paths: &[(&str, fn(&WebUIProtocol, &Value, usize) -> usize)] = &[
        (
            "string",
            run_string as fn(&WebUIProtocol, &Value, usize) -> usize,
        ),
        ("streaming", run_streaming),
        ("streaming+inject(opts)", run_streaming_with_inject),
        ("string+postinject", run_string_postinject),
    ];

    let mut rows: Vec<Row> = Vec::new();

    for &scale in &scales {
        let state = build_state(scale);
        let output_size = warmup_output_size(&protocol, &state);
        for (label, f) in paths {
            let m = measure(ITERS_PER_SCALE, 0, || {
                std::hint::black_box(f(&protocol, &state, output_size));
            });
            rows.push(Row::from_measurement(
                &format!("{label}/{scale}"),
                output_size,
                m,
            ));
        }
        // Pooled path measured separately because the closure needs to
        // capture the shared pool (can't use a fn pointer).
        let m = measure(ITERS_PER_SCALE, 0, || {
            std::hint::black_box(run_streaming_pooled_with_inject(
                &protocol,
                &state,
                output_size,
                &pool,
            ));
        });
        rows.push(Row::from_measurement(
            &format!("streaming+inject(opts) POOLED/{scale}"),
            output_size,
            m,
        ));
    }

    render_table(&rows);

    println!();
    println!("Notes:");
    println!("  * `allocs/run` and `bytes/run` are exact (custom GlobalAlloc).");
    if cfg!(windows) {
        println!("  * `user µs/run` is `GetProcessTimes` user time delta / iters.");
        println!("  * `process RSS` is `PeakWorkingSetSize` for the process at");
    } else {
        println!("  * `user µs/run` is `getrusage(RUSAGE_SELF).ru_utime` delta / iters.");
        println!("  * `process RSS` is the high-water mark for the whole process at");
    }
    println!("    phase end. Per-iteration RSS is not directly observable; use");
    println!("    `bytes/run` to compare per-render heap pressure across paths.");

    match mode {
        Mode::Print => {}
        Mode::Save(name) => baseline::save(KIND, &name, SCHEMA, rows),
        Mode::Compare(name) => {
            let _ = baseline::compare(KIND, &name, SCHEMA, &rows, 5.0);
        }
    }
}
