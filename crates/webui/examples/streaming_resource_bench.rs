// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Memory + CPU benchmark for the SSR render paths (commit 1: baseline-only).
//!
//! Measures **per-render resource usage** — allocations, bytes allocated,
//! user CPU time, peak RSS — for the two render paths that exist on
//! `origin/main`:
//!
//! 1. `string`            — pre-allocated `String` buffer (the default
//!    `ResponseWriter` pattern most hosts use today).
//! 2. `string+postinject` — `string` followed by a case-insensitive
//!    byte-window scan for `</body>` + concatenation into a fresh
//!    `String`. Mirrors the legacy dev-server livereload pipeline
//!    (`lr.inject(&buf)`) and matches what any host has to do to
//!    splice a per-request `<script>` before `</body>` without a
//!    structured injection API.
//!
//! Later commits in this branch add `streaming` and
//! `streaming+inject(opts)` rows once the streaming primitive and the
//! signal-based injection API land. The bench supports baseline save
//! / compare so the BEFORE numbers captured here can be compared
//! against the AFTER numbers from later commits:
//!
//! ```sh
//! # On this commit: save baseline
//! cargo run --release --example streaming_resource_bench -p microsoft-webui -- --save before
//! # Later commit: diff
//! cargo run --release --example streaming_resource_bench -p microsoft-webui -- --compare before
//! ```
//!
//! Baselines live at `target/bench-baselines/resource-<name>.json`.

#![allow(missing_docs)]
// SAFETY EXEMPTION: this is a benchmarking example, not library code.
// The custom `GlobalAlloc` forwards to the system allocator with the
// same layout it received; `libc::getrusage` is given a fully-zeroed,
// stack-allocated `rusage` struct. The workspace `unsafe_code = "deny"`
// lint applies to production library code; benchmarking infra is
// exempted at the file level.
#![allow(unsafe_code)]

use serde_json::{json, Value};
use std::alloc::{GlobalAlloc, Layout, System};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use webui::{build, BuildOptions, CssStrategy, DomStrategy, ResponseWriter, WebUIHandler};
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

// ── getrusage helpers ─────────────────────────────────────────────────

#[derive(Copy, Clone)]
struct Rusage {
    user_cpu: Duration,
    sys_cpu: Duration,
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

// ── State + protocol ──────────────────────────────────────────────────

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
        dom: DomStrategy::Shadow,
        plugin: None,
        components: Vec::new(),
    })
    .expect("failed to build contact-book-manager protocol")
    .protocol
}

// Body inject script used by `string+postinject` — mirrors the legacy
// dev-mode livereload pipeline. Subsequent commits introduce a
// signal-based alternative that this baseline can be compared against.
const BODY_INJECT: &str = r#"<script>(function(){var e=new EventSource('/__webui/livereload');e.addEventListener('reload',function(){location.reload()})})();</script>"#;

// ── Writers + post-inject ─────────────────────────────────────────────

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

/// Case-insensitive `</body>` byte-window scan + concat. Allocates one
/// fresh `String` for the merged output. This is the cost of every
/// per-request HTML inject when no structured injection API is
/// available — the path origin/main hosts have to take today.
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
    // Warm up: first runs are dominated by lazy initialisations.
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

// ── Snapshot save / compare ───────────────────────────────────────────

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

fn baseline_path(name: &str) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dir = manifest
        .join("..")
        .join("..")
        .join("target")
        .join("bench-baselines");
    std::fs::create_dir_all(&dir).expect("create bench-baselines dir");
    dir.join(format!("resource-{name}.json"))
}

fn save_snapshot(name: &str, rows: &[SnapshotRow]) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let snap = Snapshot {
        schema: 1,
        name: name.to_string(),
        timestamp_unix: now,
        rows: rows
            .iter()
            .map(|r| SnapshotRow {
                label: r.label.clone(),
                iters: r.iters,
                allocs_per_run: r.allocs_per_run,
                bytes_per_run: r.bytes_per_run,
                user_cpu_us_per_run: r.user_cpu_us_per_run,
                sys_cpu_us_per_run: r.sys_cpu_us_per_run,
                wall_us_per_run: r.wall_us_per_run,
                rss_high_water_bytes: r.rss_high_water_bytes,
            })
            .collect(),
    };
    let p = baseline_path(name);
    let bytes = serde_json::to_vec_pretty(&snap).expect("serialize snapshot");
    std::fs::write(&p, bytes).expect("write snapshot");
    println!("\n✔ Baseline saved to {}", p.display());
}

fn load_snapshot(name: &str) -> Option<Snapshot> {
    let p = baseline_path(name);
    if !p.exists() {
        eprintln!(
            "\n⚠ baseline '{}' not found at {} — run with --save first",
            name,
            p.display()
        );
        return None;
    }
    let raw = std::fs::read(&p).ok()?;
    serde_json::from_slice::<Snapshot>(&raw).ok()
}

fn print_diff(current: &[SnapshotRow], baseline: &Snapshot) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mins_old = now.saturating_sub(baseline.timestamp_unix) / 60;
    let age_label = match mins_old {
        0 => "<1m ago".to_string(),
        1..=59 => format!("{mins_old}m ago"),
        60..=1439 => format!("{}h ago", mins_old / 60),
        _ => format!("{}d ago", mins_old / 1440),
    };
    println!(
        "\nDiff vs baseline '{}' (saved {})",
        baseline.name, age_label
    );
    println!(
        "| {:<42} | {:>14} | {:>14} | {:>14} |",
        "row", "allocs Δ%", "bytes Δ%", "user_cpu Δ%"
    );
    println!("|{:-<44}|{:->16}|{:->16}|{:->16}|", "", "", "", "");

    let baseline_by_label: std::collections::HashMap<&str, &SnapshotRow> = baseline
        .rows
        .iter()
        .map(|r| (r.label.as_str(), r))
        .collect();

    for row in current {
        let label = row.label.as_str();
        if let Some(base) = baseline_by_label.get(label) {
            let pct = |old: f64, new: f64| -> String {
                if old == 0.0 {
                    "—".to_string()
                } else {
                    let d = (new - old) / old * 100.0;
                    format!("{d:>13.1}%")
                }
            };
            println!(
                "| {:<42} | {:>14} | {:>14} | {:>14} |",
                label,
                pct(base.allocs_per_run, row.allocs_per_run),
                pct(base.bytes_per_run, row.bytes_per_run),
                pct(base.user_cpu_us_per_run, row.user_cpu_us_per_run),
            );
        } else {
            println!(
                "| {:<42} | {:>14} | {:>14} | {:>14} |",
                label, "(new row)", "—", "—"
            );
        }
    }
    println!("\nNegative Δ% = improvement; positive = regression. Threshold for action: ±5%.");
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

// ── CLI args ──────────────────────────────────────────────────────────

enum Mode {
    Print,
    Save(String),
    Compare(String),
}

fn parse_args() -> Mode {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--save" => {
                let name = args.next().unwrap_or_else(|| {
                    eprintln!("--save requires a name");
                    std::process::exit(2);
                });
                return Mode::Save(name);
            }
            "--compare" => {
                let name = args.next().unwrap_or_else(|| {
                    eprintln!("--compare requires a name");
                    std::process::exit(2);
                });
                return Mode::Compare(name);
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

    println!("WebUI SSR resource benchmark (commit 1: baseline paths only)");
    println!("============================================================");
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

    let paths: &[(&str, fn(&WebUIProtocol, &Value, usize) -> usize)] = &[
        (
            "string",
            run_string as fn(&WebUIProtocol, &Value, usize) -> usize,
        ),
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
