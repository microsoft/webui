// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! CPU-time benchmark for the JSON-state hot path of the SSR pipeline.
//!
//! Load tests reported the Node.js SSR host burning ~60% CPU vs ~25% for the
//! control at 150 RPS, with traces pointing at JSON processing of the **state
//! object**. Every request the native addon serves pays, in order:
//!
//! 1. `serde_json::from_str::<Value>(state_json)` — build a full owned tree
//!    (`BTreeMap` per object, `String` per key/value, `Vec` per array).
//! 2. `WebUIHandler::render(...)` — walk the protocol, resolving signals /
//!    loops / conditions against that tree.
//!
//! Criterion reports wall-clock only. This example reports **CPU time and CPU%**
//! (user + system, from `getrusage` on Unix / `GetProcessTimes` on Windows) so
//! we can see how CPU-bound each stage is and how much of the per-request CPU is
//! spent *just parsing state* before any HTML is produced. It isolates three
//! stages at increasing state sizes:
//!
//! * `parse`         — `serde_json::from_str` only (the addon's step 1).
//! * `render`        — `handle()` on a pre-parsed `Value` (step 2).
//! * `parse+render`  — parse **then** render (the real per-request FFI cost,
//!    minus napi string marshalling — see the `ffi_cpu_bench.mjs` Node harness
//!    for the measurement that includes the FFI boundary).
//!
//! `parse+render − render ≈ parse CPU`, so the gap between those two rows is the
//! CPU the traces are blaming on JSON.
//!
//! For each stage × scale it reports allocations, bytes, wall µs/op, **user
//! µs/op**, **sys µs/op**, **CPU%** (`(user+sys)/wall`), and state throughput
//! (MiB of input JSON parsed per second).
//!
//! Usage:
//!
//! ```sh
//! cargo run --release --example state_cpu_bench -p microsoft-webui
//! cargo run --release --example state_cpu_bench -p microsoft-webui -- --save main
//! cargo run --release --example state_cpu_bench -p microsoft-webui -- --compare main
//! # or via xtask:
//! cargo xtask bench state-cpu
//! cargo xtask bench state-cpu --save-baseline main
//! cargo xtask bench state-cpu --baseline main
//! ```

#![allow(missing_docs)]
// SAFETY EXEMPTION: benchmark example, not library code. `GlobalAlloc` and the
// process resource APIs (`getrusage` / `GetProcessTimes`) require `unsafe`; the
// callers here uphold their contracts (System allocator forwarding with the
// original layout; resource structs fully initialised before the FFI call). The
// workspace `unsafe_code = "deny"` lint targets production library code; this
// benchmarking binary is exempted at the file level.
#![allow(unsafe_code)]

use serde_json::{json, Value};
use std::alloc::{GlobalAlloc, Layout, System};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
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

// ── Process CPU helpers ───────────────────────────────────────────────

#[derive(Copy, Clone)]
struct ProcessUsage {
    user_cpu: Duration,
    sys_cpu: Duration,
}

impl ProcessUsage {
    #[cfg(unix)]
    fn now() -> Self {
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        // SAFETY: `usage` is a valid mutable pointer to a fully-initialised
        // (zeroed) rusage struct; getrusage(2) writes to it.
        let rc = unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
        assert_eq!(rc, 0, "getrusage failed");
        Self {
            user_cpu: timeval_to_duration(usage.ru_utime),
            sys_cpu: timeval_to_duration(usage.ru_stime),
        }
    }

    #[cfg(windows)]
    fn now() -> Self {
        use windows_sys::Win32::Foundation::FILETIME;
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

        // GetCurrentProcess returns a pseudo-handle owned by the process; it
        // must not be closed.
        let process = unsafe { GetCurrentProcess() };
        let mut creation_time = FILETIME::default();
        let mut exit_time = FILETIME::default();
        let mut kernel_time = FILETIME::default();
        let mut user_time = FILETIME::default();
        // SAFETY: all pointers refer to writable FILETIME values and the
        // pseudo-handle returned by GetCurrentProcess is valid for this call.
        let times_ok = unsafe {
            GetProcessTimes(
                process,
                &mut creation_time,
                &mut exit_time,
                &mut kernel_time,
                &mut user_time,
            )
        };
        assert_ne!(times_ok, 0, "GetProcessTimes failed");

        Self {
            user_cpu: filetime_to_duration(user_time),
            sys_cpu: filetime_to_duration(kernel_time),
        }
    }

    #[cfg(not(any(unix, windows)))]
    fn now() -> Self {
        Self {
            user_cpu: Duration::ZERO,
            sys_cpu: Duration::ZERO,
        }
    }
}

#[cfg(unix)]
fn timeval_to_duration(tv: libc::timeval) -> Duration {
    let secs = tv.tv_sec as u64;
    let usecs = tv.tv_usec as u32;
    Duration::new(secs, usecs * 1_000)
}

#[cfg(windows)]
fn filetime_to_duration(filetime: windows_sys::Win32::Foundation::FILETIME) -> Duration {
    let ticks = (u64::from(filetime.dwHighDateTime) << 32) | u64::from(filetime.dwLowDateTime);
    let secs = ticks / 10_000_000;
    let nanos = match u32::try_from((ticks % 10_000_000) * 100) {
        Ok(value) => value,
        Err(_) => panic!("FILETIME nanosecond remainder must fit in u32"),
    };
    Duration::new(secs, nanos)
}

// ── Measurement ───────────────────────────────────────────────────────

#[derive(Copy, Clone)]
struct CpuDelta {
    iters: usize,
    allocs: usize,
    bytes: usize,
    user_cpu: Duration,
    sys_cpu: Duration,
    wall_time: Duration,
    /// Bytes of input state JSON processed per iteration (throughput axis).
    state_bytes: usize,
}

struct PerIter {
    allocs: f64,
    bytes: f64,
    user_cpu_us: f64,
    sys_cpu_us: f64,
    wall_us: f64,
    /// (user + sys) / wall * 100 — how CPU-bound the stage is.
    cpu_pct: f64,
    /// Input state JSON throughput in MiB/s.
    state_mib_s: f64,
}

impl CpuDelta {
    fn per_iter(&self) -> PerIter {
        let n = self.iters as f64;
        let user_us = self.user_cpu.as_secs_f64() * 1_000_000.0 / n;
        let sys_us = self.sys_cpu.as_secs_f64() * 1_000_000.0 / n;
        let wall_us = self.wall_time.as_secs_f64() * 1_000_000.0 / n;
        let cpu_pct = if wall_us > 0.0 {
            (user_us + sys_us) / wall_us * 100.0
        } else {
            0.0
        };
        let wall_s = self.wall_time.as_secs_f64() / n;
        let state_mib_s = if wall_s > 0.0 {
            (self.state_bytes as f64 / (1024.0 * 1024.0)) / wall_s
        } else {
            0.0
        };
        PerIter {
            allocs: self.allocs as f64 / n,
            bytes: self.bytes as f64 / n,
            user_cpu_us: user_us,
            sys_cpu_us: sys_us,
            wall_us,
            cpu_pct,
            state_mib_s,
        }
    }
}

fn measure<F>(iters: usize, state_bytes: usize, mut f: F) -> CpuDelta
where
    F: FnMut(),
{
    // Warm up: first runs are dominated by lazy initialisations
    // (formatter caches, allocator slabs, etc.).
    for _ in 0..3 {
        f();
    }

    let (a0, b0) = alloc_snapshot();
    let r0 = ProcessUsage::now();
    let t0 = Instant::now();

    for _ in 0..iters {
        f();
    }

    let wall = t0.elapsed();
    let r1 = ProcessUsage::now();
    let (a1, b1) = alloc_snapshot();

    CpuDelta {
        iters,
        allocs: a1.saturating_sub(a0),
        bytes: b1.saturating_sub(b0),
        user_cpu: r1.user_cpu.saturating_sub(r0.user_cpu),
        sys_cpu: r1.sys_cpu.saturating_sub(r0.sys_cpu),
        wall_time: wall,
        state_bytes,
    }
}

/// Pick an iteration count so the stage runs for roughly `target` wall time.
///
/// Cheap stages (a ~30 µs render) need thousands of iterations to exceed the
/// OS CPU-clock resolution (≈15.6 ms per `GetProcessTimes` tick on Windows),
/// while an expensive stage (a ~34 ms parse of 10k items) needs only a few
/// dozen. A fixed count can't serve both without either being noisy or slow,
/// so we probe the per-iter cost once and size the run to the time budget.
fn calibrate<F>(target: Duration, min: usize, max: usize, f: &mut F) -> usize
where
    F: FnMut(),
{
    for _ in 0..3 {
        f();
    }
    let probe = 16usize;
    let t0 = Instant::now();
    for _ in 0..probe {
        f();
    }
    let per = t0.elapsed().as_secs_f64() / probe as f64;
    if per <= 0.0 {
        return max;
    }
    let want = target.as_secs_f64() / per;
    // `want` is a positive, finite count derived from a time ratio.
    let want = if want.is_finite() && want >= 1.0 {
        want as usize
    } else {
        min
    };
    want.clamp(min, max)
}

/// Calibrate the iteration count to `target`, then measure the stage.
fn bench_stage<F>(target: Duration, state_bytes: usize, mut f: F) -> CpuDelta
where
    F: FnMut(),
{
    let iters = calibrate(target, 50, 200_000, &mut f);
    measure(iters, state_bytes, f)
}

// ── State + protocol builders (contact-book-manager app) ──────────────

const FIRST_NAMES: &[&str] = &[
    "Ava", "Liam", "Noah", "Emma", "Mia", "Ethan", "Sofia", "Lucas", "Aria", "Mateo",
];
const LAST_NAMES: &[&str] = &[
    "Nguyen",
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
        "favorite": idx % 3 == 0,
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

// ── Stage drivers ─────────────────────────────────────────────────────

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

/// Stage 1: parse the state JSON into an owned `serde_json::Value` tree.
fn run_parse(state_json: &str) -> usize {
    let value: Value = serde_json::from_str(state_json).expect("state parse");
    std::hint::black_box(&value);
    // Return a cheap size proxy so the optimiser cannot elide the parse.
    match &value {
        Value::Object(map) => map.len(),
        _ => 0,
    }
}

/// Stage 2: render a pre-parsed state tree (no parse cost).
fn run_render(protocol: &WebUIProtocol, state: &Value, output_cap: usize) -> usize {
    let handler = WebUIHandler::new();
    let mut writer = StringWriter::with_capacity(output_cap);
    handler
        .render(
            protocol,
            state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .expect("render");
    writer.buf.len()
}

/// The real per-request cost: parse the state JSON, then render it. This is
/// what the native addon's `render()` does on every request (minus the napi
/// string marshalling captured by the Node harness).
fn run_parse_render(protocol: &WebUIProtocol, state_json: &str, output_cap: usize) -> usize {
    let state: Value = serde_json::from_str(state_json).expect("state parse");
    let handler = WebUIHandler::new();
    let mut writer = StringWriter::with_capacity(output_cap);
    handler
        .render(
            protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .expect("render");
    writer.buf.len()
}

// ── Reporting ─────────────────────────────────────────────────────────

fn print_header() {
    println!();
    println!(
        "| {:<22} | {:>7} | {:>10} | {:>11} | {:>9} | {:>11} | {:>10} | {:>6} | {:>11} |",
        "stage / scale",
        "iters",
        "allocs/op",
        "bytes/op",
        "wall µs",
        "user µs/op",
        "sys µs/op",
        "CPU%",
        "state MiB/s",
    );
    println!(
        "|{:-<24}|{:->9}|{:->12}|{:->13}|{:->11}|{:->13}|{:->12}|{:->8}|{:->13}|",
        "", "", "", "", "", "", "", "", ""
    );
}

fn print_row(label: &str, delta: CpuDelta) {
    let pi = delta.per_iter();
    println!(
        "| {:<22} | {:>7} | {:>10.2} | {:>11} | {:>9.2} | {:>11.2} | {:>10.2} | {:>5.0}% | {:>11.1} |",
        label,
        delta.iters,
        pi.allocs,
        format_bytes_per_op(pi.bytes),
        pi.wall_us,
        pi.user_cpu_us,
        pi.sys_cpu_us,
        pi.cpu_pct,
        pi.state_mib_s,
    );
}

fn format_bytes_per_op(bytes: f64) -> String {
    if bytes < 1024.0 {
        format!("{bytes:.0} B")
    } else if bytes < 1024.0 * 1024.0 {
        format!("{:.1} KiB", bytes / 1024.0)
    } else {
        format!("{:.2} MiB", bytes / (1024.0 * 1024.0))
    }
}

// ── Snapshot serialization ────────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize)]
struct SnapshotRow {
    label: String,
    iters: usize,
    allocs_per_op: f64,
    bytes_per_op: f64,
    user_cpu_us_per_op: f64,
    sys_cpu_us_per_op: f64,
    wall_us_per_op: f64,
    cpu_pct: f64,
    state_mib_s: f64,
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
        .join(format!("state-cpu-{name}.json"))
}

fn delta_to_row(label: &str, delta: CpuDelta) -> SnapshotRow {
    let pi = delta.per_iter();
    SnapshotRow {
        label: label.to_string(),
        iters: delta.iters,
        allocs_per_op: pi.allocs,
        bytes_per_op: pi.bytes,
        user_cpu_us_per_op: pi.user_cpu_us,
        sys_cpu_us_per_op: pi.sys_cpu_us,
        wall_us_per_op: pi.wall_us,
        cpu_pct: pi.cpu_pct,
        state_mib_s: pi.state_mib_s,
    }
}

fn save_snapshot(name: &str, rows: Vec<SnapshotRow>) {
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
        rows,
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

fn print_diff(current: &[SnapshotRow], baseline: &Snapshot) {
    println!();
    println!(
        "Diff vs baseline '{}' (saved {} ago)",
        baseline.name,
        format_age(baseline.timestamp_unix)
    );
    println!(
        "| {:<22} | {:>14} | {:>14} | {:>14} |",
        "row", "user_cpu Δ%", "wall Δ%", "bytes Δ%"
    );
    println!("|{:-<24}|{:->16}|{:->16}|{:->16}|", "", "", "", "");
    for cur in current {
        match baseline.rows.iter().find(|b| b.label == cur.label) {
            Some(base) => {
                println!(
                    "| {:<22} | {:>13.1}% | {:>13.1}% | {:>13.1}% |",
                    cur.label,
                    pct_change(base.user_cpu_us_per_op, cur.user_cpu_us_per_op),
                    pct_change(base.wall_us_per_op, cur.wall_us_per_op),
                    pct_change(base.bytes_per_op, cur.bytes_per_op),
                );
            }
            None => {
                println!(
                    "| {:<22} | {:>14} | {:>14} | {:>14} |",
                    cur.label, "(new row)", "—", "—"
                );
            }
        }
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

// ── CLI ───────────────────────────────────────────────────────────────

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
                    "Usage: state_cpu_bench [--save NAME] [--compare NAME]\n\n\
                     With no args: prints the CPU table.\n\
                     --save NAME: write current results to target/bench-baselines/state-cpu-NAME.json\n\
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
    // Larger scales than the wall-clock benches: the whole point is to expose
    // the state-parse CPU that dominates at big JSON payloads.
    let scales = [1_000usize, 5_000, 10_000];
    // Per-row wall-time budget. Iteration counts are calibrated to this so the
    // cheap `render` stage runs enough times to clear the OS CPU-clock tick.
    let budget = Duration::from_millis(1_500);

    println!("WebUI state-object CPU benchmark (parse vs render vs parse+render)");
    println!("=================================================================");
    println!(
        "Build: {} | per-row budget: {} ms (iterations auto-calibrated)",
        if cfg!(debug_assertions) {
            "DEBUG (numbers will be misleading; rebuild with --release)"
        } else {
            "release"
        },
        budget.as_millis()
    );
    println!("CPU% = (user+sys)/wall. ~100% ⇒ pure compute. `parse+render − render` ≈ parse CPU.");
    print_header();

    let protocol = build_protocol();
    let mut snapshot_rows: Vec<SnapshotRow> = Vec::new();

    for &scale in &scales {
        let state = build_state(scale);
        let state_json = serde_json::to_string(&state).expect("serialize state");
        let state_bytes = state_json.len();
        // Size the render output buffer once so per-op growth doesn't skew CPU.
        let output_cap = {
            let mut w = StringWriter::with_capacity(256 * 1024);
            WebUIHandler::new()
                .render(
                    &protocol,
                    &state,
                    &RenderOptions::new("index.html", "/"),
                    &mut w,
                )
                .expect("warmup render");
            w.buf.len()
        };

        // parse
        let delta = bench_stage(budget, state_bytes, || {
            std::hint::black_box(run_parse(&state_json));
        });
        let label = format!("parse/{scale}");
        print_row(&label, delta);
        snapshot_rows.push(delta_to_row(&label, delta));

        // render (pre-parsed)
        let delta = bench_stage(budget, state_bytes, || {
            std::hint::black_box(run_render(&protocol, &state, output_cap));
        });
        let label = format!("render/{scale}");
        print_row(&label, delta);
        snapshot_rows.push(delta_to_row(&label, delta));

        // parse + render (the real per-request cost)
        let delta = bench_stage(budget, state_bytes, || {
            std::hint::black_box(run_parse_render(&protocol, &state_json, output_cap));
        });
        let label = format!("parse+render/{scale}");
        print_row(&label, delta);
        snapshot_rows.push(delta_to_row(&label, delta));

        println!(
            "|{:-<24}|{:->9}|{:->12}|{:->13}|{:->11}|{:->13}|{:->12}|{:->8}|{:->13}|",
            "", "", "", "", "", "", "", "", ""
        );
    }

    println!();
    println!("Notes:");
    println!("  * `allocs/op` and `bytes/op` are exact (custom GlobalAlloc).");
    if cfg!(windows) {
        println!("  * `user µs/op` / `sys µs/op` are `GetProcessTimes` deltas / iters.");
    } else {
        println!("  * `user µs/op` / `sys µs/op` are `getrusage(RUSAGE_SELF)` deltas / iters.");
    }
    println!("  * `state MiB/s` normalizes throughput to the input state-JSON size,");
    println!("    so rows at the same scale are directly comparable.");
    println!("  * The FFI boundary (napi string copy + per-chunk callback) is measured");
    println!("    separately by crates/webui-node/bench/ffi_cpu_bench.mjs.");

    match mode {
        Mode::Print => {}
        Mode::Save(name) => save_snapshot(&name, snapshot_rows),
        Mode::Compare(name) => {
            if let Some(baseline) = load_snapshot(&name) {
                print_diff(&snapshot_rows, &baseline);
            }
        }
    }
}
