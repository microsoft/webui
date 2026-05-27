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
//! For each path × scale (10 / 100 / 1000 contacts) it reports:
//!
//! * **allocations**  — count of `alloc` calls (custom GlobalAlloc)
//! * **bytes allocated** — total bytes requested
//! * **CPU user time** — `getrusage(RUSAGE_SELF).ru_utime` delta
//! * **peak RSS** — `ru_maxrss` high-water mark
//!
//! Unlike criterion (which only reports wall-clock), this gives a
//! direct allocator-level view useful for verifying that the streaming
//! writer's "zero per-write allocation" claim actually holds in the
//! production path.
//!
//! Usage:
//!
//! ```sh
//! cargo run --release --example streaming_resource_bench -p microsoft-webui
//! ```

#![allow(missing_docs)]
// SAFETY EXEMPTION: This is a benchmark example, not library code.
// `GlobalAlloc` and `libc::getrusage` require `unsafe` blocks; their
// callers here have correct contracts (forwarding to System allocator
// with original layouts; `rusage` is fully zero-initialised before the
// FFI call). The workspace `unsafe_code = "deny"` lint applies to
// production library code; benchmarking infrastructure is exempted at
// the file level with this attribute.
#![allow(unsafe_code)]

use bytes::Bytes;
use serde_json::{json, Value};
use std::alloc::{GlobalAlloc, Layout, System};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use webui::streaming::{ChunkPool, StreamingWriter};
use webui::{build, BuildOptions, CssStrategy, ResponseWriter, WebUIHandler};
use webui_handler::RenderOptions;
use webui_protocol::WebUIProtocol;

// ── Counting allocator ────────────────────────────────────────────────

struct CountingAlloc;

static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);
static ALLOC_BYTES: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        // SAFETY: forwarded with the same layout the caller produced.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwarded; ptr/layout came from `alloc` above.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        // SAFETY: forwarded with the same layout the caller produced.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // Realloc to a strictly larger size counts as one new allocation
        // for the size delta — matches what most heap profilers do.
        if new_size > layout.size() {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
            ALLOC_BYTES.fetch_add(new_size - layout.size(), Ordering::Relaxed);
        }
        // SAFETY: forwarded; ptr/layout came from `alloc` above.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn alloc_snapshot() -> (usize, usize) {
    (
        ALLOC_COUNT.load(Ordering::Relaxed),
        ALLOC_BYTES.load(Ordering::Relaxed),
    )
}

// ── getrusage helpers ─────────────────────────────────────────────────

#[derive(Copy, Clone)]
struct Rusage {
    user_cpu: Duration,
    sys_cpu: Duration,
    /// Maximum resident set size, in bytes (macOS) or KB (Linux).
    /// Normalised by `max_rss_bytes`.
    max_rss_raw: i64,
}

impl Rusage {
    fn now() -> Self {
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        // SAFETY: `usage` is a valid mutable pointer to a fully-initialised
        // (zeroed) rusage struct; getrusage(2) writes to it.
        let rc = unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
        assert_eq!(rc, 0, "getrusage failed");
        Self {
            user_cpu: timeval_to_duration(usage.ru_utime),
            sys_cpu: timeval_to_duration(usage.ru_stime),
            max_rss_raw: usage.ru_maxrss as i64,
        }
    }

    fn max_rss_bytes(&self) -> i64 {
        if cfg!(target_os = "macos") {
            self.max_rss_raw
        } else {
            self.max_rss_raw * 1024
        }
    }
}

fn timeval_to_duration(tv: libc::timeval) -> Duration {
    let secs = tv.tv_sec as u64;
    let usecs = tv.tv_usec as u32;
    Duration::new(secs, usecs * 1_000)
}

#[derive(Copy, Clone)]
struct ResourceDelta {
    iters: usize,
    allocs: usize,
    bytes: usize,
    user_cpu: Duration,
    sys_cpu: Duration,
    wall_time: Duration,
    rss_high_water_bytes: i64,
}

impl ResourceDelta {
    fn per_iter(&self) -> PerIter {
        let n = self.iters as f64;
        PerIter {
            allocs: self.allocs as f64 / n,
            bytes: self.bytes as f64 / n,
            user_cpu_us: self.user_cpu.as_secs_f64() * 1_000_000.0 / n,
            sys_cpu_us: self.sys_cpu.as_secs_f64() * 1_000_000.0 / n,
            wall_us: self.wall_time.as_secs_f64() * 1_000_000.0 / n,
            rss_bytes: self.rss_high_water_bytes,
        }
    }
}

struct PerIter {
    allocs: f64,
    bytes: f64,
    user_cpu_us: f64,
    sys_cpu_us: f64,
    wall_us: f64,
    rss_bytes: i64,
}

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

// ── Measurement loop ──────────────────────────────────────────────────

fn measure<F>(iters: usize, mut f: F) -> ResourceDelta
where
    F: FnMut(),
{
    // Warm up: first runs are dominated by lazy initialisations
    // (formatter caches, allocator slabs, etc.).
    for _ in 0..3 {
        f();
    }

    let (a0, b0) = alloc_snapshot();
    let r0 = Rusage::now();
    let t0 = Instant::now();

    for _ in 0..iters {
        f();
    }

    let wall = t0.elapsed();
    let r1 = Rusage::now();
    let (a1, b1) = alloc_snapshot();

    ResourceDelta {
        iters,
        allocs: a1.saturating_sub(a0),
        bytes: b1.saturating_sub(b0),
        user_cpu: r1.user_cpu.saturating_sub(r0.user_cpu),
        sys_cpu: r1.sys_cpu.saturating_sub(r0.sys_cpu),
        wall_time: wall,
        rss_high_water_bytes: r1.max_rss_bytes(),
    }
}

// ── Reporting ─────────────────────────────────────────────────────────

fn print_header() {
    println!();
    println!(
        "| {:<26} | {:>7} | {:>10} | {:>13} | {:>9} | {:>11} | {:>10} | {:>14} |",
        "path/scale (output bytes)",
        "iters",
        "allocs/run",
        "bytes/run",
        "wall µs",
        "user µs/run",
        "sys µs/run",
        "process RSS",
    );
    println!(
        "|{:-<28}|{:->9}|{:->12}|{:->15}|{:->11}|{:->13}|{:->12}|{:->16}|",
        "", "", "", "", "", "", "", ""
    );
}

fn print_row(label: &str, delta: ResourceDelta) {
    let pi = delta.per_iter();
    println!(
        "| {:<26} | {:>7} | {:>10.2} | {:>13} | {:>9.2} | {:>11.2} | {:>10.2} | {:>14} |",
        label,
        delta.iters,
        pi.allocs,
        format_bytes_per_run(pi.bytes),
        pi.wall_us,
        pi.user_cpu_us,
        pi.sys_cpu_us,
        format_total_rss(pi.rss_bytes),
    );
}

fn format_bytes_per_run(bytes: f64) -> String {
    if bytes < 1024.0 {
        format!("{bytes:.0} B")
    } else if bytes < 1024.0 * 1024.0 {
        format!("{:.1} KiB", bytes / 1024.0)
    } else {
        format!("{:.2} MiB", bytes / (1024.0 * 1024.0))
    }
}

fn format_total_rss(bytes: i64) -> String {
    if bytes < 1024 * 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{:.2} MiB", bytes as f64 / (1024.0 * 1024.0))
    }
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

// ── Snapshot serialization ────────────────────────────────────────────

/// One row of the bench, in JSON-friendly form (no formatters).
#[derive(serde::Serialize, serde::Deserialize)]
struct SnapshotRow {
    label: String,
    iters: usize,
    allocs_per_run: f64,
    bytes_per_run: f64,
    user_cpu_us_per_run: f64,
    sys_cpu_us_per_run: f64,
    wall_us_per_run: f64,
    rss_high_water_bytes: i64,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Snapshot {
    schema: u32,
    name: String,
    timestamp_unix: u64,
    rows: Vec<SnapshotRow>,
}

const SNAPSHOT_SCHEMA: u32 = 1;

fn snapshot_path(name: &str) -> std::path::PathBuf {
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("..")
        .join("..")
        .join("target")
        .join("bench-baselines")
        .join(format!("resource-{name}.json"))
}

fn save_snapshot(name: &str, rows: &[SnapshotRow]) {
    let path = snapshot_path(name);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let snap = Snapshot {
        schema: SNAPSHOT_SCHEMA,
        name: name.to_string(),
        timestamp_unix: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        rows: rows.iter().map(SnapshotRow::clone_data).collect(),
    };
    let json = match serde_json::to_string_pretty(&snap) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("snapshot: serialize failed: {e}");
            return;
        }
    };
    if let Err(e) = std::fs::write(&path, json) {
        eprintln!("snapshot: write {} failed: {e}", path.display());
        return;
    }
    println!();
    println!("✔ Baseline saved to {}", path.display());
}

fn load_snapshot(name: &str) -> Option<Snapshot> {
    let path = snapshot_path(name);
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => {
            eprintln!(
                "compare: baseline '{name}' not found at {} — run with --save {name} first",
                path.display()
            );
            return None;
        }
    };
    match serde_json::from_slice::<Snapshot>(&bytes) {
        Ok(s) if s.schema == SNAPSHOT_SCHEMA => Some(s),
        Ok(s) => {
            eprintln!(
                "compare: baseline '{name}' has schema {} (expected {SNAPSHOT_SCHEMA}); regenerate with --save",
                s.schema
            );
            None
        }
        Err(e) => {
            eprintln!("compare: parse {} failed: {e}", path.display());
            None
        }
    }
}

impl SnapshotRow {
    fn clone_data(&self) -> SnapshotRow {
        SnapshotRow {
            label: self.label.clone(),
            iters: self.iters,
            allocs_per_run: self.allocs_per_run,
            bytes_per_run: self.bytes_per_run,
            user_cpu_us_per_run: self.user_cpu_us_per_run,
            sys_cpu_us_per_run: self.sys_cpu_us_per_run,
            wall_us_per_run: self.wall_us_per_run,
            rss_high_water_bytes: self.rss_high_water_bytes,
        }
    }
}

fn print_diff(current: &[SnapshotRow], baseline: &Snapshot) {
    println!();
    println!(
        "Diff vs baseline '{}' (saved {} ago)",
        baseline.name,
        format_age(baseline.timestamp_unix)
    );
    println!(
        "| {:<42} | {:>14} | {:>14} | {:>14} |",
        "row", "allocs Δ%", "bytes Δ%", "user_cpu Δ%"
    );
    println!("|{:-<44}|{:->16}|{:->16}|{:->16}|", "", "", "", "");
    for cur in current {
        let base = baseline.rows.iter().find(|b| b.label == cur.label);
        let (a, b, c) = match base {
            Some(base) => (
                pct_change(base.allocs_per_run, cur.allocs_per_run),
                pct_change(base.bytes_per_run, cur.bytes_per_run),
                pct_change(base.user_cpu_us_per_run, cur.user_cpu_us_per_run),
            ),
            None => {
                println!(
                    "| {:<42} | {:>14} | {:>14} | {:>14} |",
                    cur.label, "(new row)", "—", "—"
                );
                continue;
            }
        };
        println!(
            "| {:<42} | {:>13.1}% | {:>13.1}% | {:>13.1}% |",
            cur.label, a, b, c
        );
    }
    println!();
    println!("Negative Δ% = improvement; positive = regression. Threshold for action: ±5%.");
    println!();
}

fn pct_change(base: f64, current: f64) -> f64 {
    if base == 0.0 {
        return 0.0;
    }
    ((current - base) / base) * 100.0
}

fn format_age(then_unix: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let secs = now.saturating_sub(then_unix);
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86400)
    }
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
    let iters_per_scale = 2_000;

    println!("WebUI streaming resource benchmark");
    println!("==================================");
    println!(
        "Build: {} | iterations per row: {}",
        if cfg!(debug_assertions) {
            "DEBUG (numbers will be misleading; rebuild with --release)"
        } else {
            "release"
        },
        iters_per_scale
    );
    println!(
        "RSS column = process-wide high-water mark observed at end of phase \
         (cumulative across all phases, only meaningful as a peak)."
    );
    print_header();

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

    let mut snapshot_rows: Vec<SnapshotRow> = Vec::new();

    for &scale in &scales {
        let state = build_state(scale);
        let output_size = warmup_output_size(&protocol, &state);
        for (label, f) in paths {
            let delta = measure(iters_per_scale, || {
                std::hint::black_box(f(&protocol, &state, output_size));
            });
            let row_label = format!("{label}/{scale}");
            print_row(&format!("{row_label} ({output_size}B)"), delta);
            snapshot_rows.push(delta_to_row(&row_label, delta));
        }
        // Pooled path measured separately because the closure needs to
        // capture the shared pool (can't use a fn pointer).
        let delta = measure(iters_per_scale, || {
            std::hint::black_box(run_streaming_pooled_with_inject(
                &protocol,
                &state,
                output_size,
                &pool,
            ));
        });
        let row_label = format!("streaming+inject(opts) POOLED/{scale}");
        print_row(&format!("{row_label} ({output_size}B)"), delta);
        snapshot_rows.push(delta_to_row(&row_label, delta));
        println!(
            "|{:-<28}|{:->9}|{:->12}|{:->15}|{:->11}|{:->13}|{:->12}|{:->16}|",
            "", "", "", "", "", "", "", ""
        );
    }
    println!();
    println!("Notes:");
    println!("  * `allocs/run` and `bytes/run` are exact (custom GlobalAlloc).");
    println!("  * `user µs/run` is `getrusage(RUSAGE_SELF).ru_utime` delta / iters.");
    println!("  * `process RSS` is the high-water mark for the whole process at");
    println!("    phase end. Per-iteration RSS is not directly observable; use");
    println!("    `bytes/run` to compare per-render heap pressure across paths.");

    match mode {
        Mode::Print => {}
        Mode::Save(name) => save_snapshot(&name, &snapshot_rows),
        Mode::Compare(name) => {
            if let Some(baseline) = load_snapshot(&name) {
                print_diff(&snapshot_rows, &baseline);
            }
        }
    }
}

fn delta_to_row(label: &str, delta: ResourceDelta) -> SnapshotRow {
    let pi = delta.per_iter();
    SnapshotRow {
        label: label.to_string(),
        iters: delta.iters,
        allocs_per_run: pi.allocs,
        bytes_per_run: pi.bytes,
        user_cpu_us_per_run: pi.user_cpu_us,
        sys_cpu_us_per_run: pi.sys_cpu_us,
        wall_us_per_run: pi.wall_us,
        rss_high_water_bytes: pi.rss_bytes,
    }
}
