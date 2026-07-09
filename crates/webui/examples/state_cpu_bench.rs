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
//! (user + system) so we can see how CPU-bound each stage is and how much of the
//! per-request CPU is spent *just parsing state* before any HTML is produced. It
//! isolates three stages at increasing state sizes:
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
//! The measurement primitives — the allocation counter, the cross-platform
//! CPU-time reader, the self-calibrating `bench`, the baseline snapshots and the
//! result table — all come from the shared [`webui_bench_support`] dev crate, so
//! this example only supplies the workload and reports the four dimensions
//! (cpu / memory / throughput / latency) the same way every resource bench does.
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

use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;
use webui::{build, BuildOptions, CssStrategy, ResponseWriter, WebUIHandler};
use webui_bench_support::report::{format_bytes, Align, Table};
use webui_bench_support::{baseline, bench, BaselineRow, CountingAllocator, Measurement, Metric};
use webui_handler::RenderOptions;
use webui_protocol::WebUIProtocol;

// The allocation counter, CPU-time reader, calibrating `bench`, baseline
// snapshots and result table all live in the shared `webui-bench-support` dev
// crate so every resource bench reports the same four dimensions the same way.
// This example only installs the counting allocator and supplies the workload.
#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator::new();

/// Baseline row schema; bump on any change to [`Row`]'s fields or metrics.
const SCHEMA: u32 = 2;
/// Baseline file kind — `target/bench-baselines/state-cpu-<name>.json`.
const KIND: &str = "state-cpu";

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

// ── Result row (measurement → report + baseline) ──────────────────────

/// One `stage/scale` result: the shared [`Measurement`] projected to per-op
/// metrics, in a shape that both prints and round-trips through a baseline.
#[derive(serde::Serialize, serde::Deserialize)]
struct Row {
    label: String,
    iters: usize,
    allocs_per_op: f64,
    bytes_per_op: f64,
    user_us: f64,
    sys_us: f64,
    wall_us: f64,
    cpu_pct: f64,
    state_mib_s: f64,
}

impl Row {
    fn from_measurement(label: &str, m: Measurement) -> Self {
        let pi = m.per_iter();
        Self {
            label: label.to_string(),
            iters: m.iters,
            allocs_per_op: pi.allocs,
            bytes_per_op: pi.bytes,
            user_us: pi.user_us,
            sys_us: pi.sys_us,
            wall_us: pi.wall_us,
            cpu_pct: pi.cpu_pct,
            state_mib_s: pi.work_mib_s,
        }
    }
}

impl BaselineRow for Row {
    fn key(&self) -> String {
        self.label.clone()
    }

    /// The four dimensions gated for regressions: cpu, memory, latency, and
    /// throughput (one metric each, in display order).
    fn metrics(&self) -> Vec<Metric> {
        vec![
            Metric::lower_better("user µs", self.user_us),
            Metric::lower_better("bytes", self.bytes_per_op),
            Metric::lower_better("wall µs", self.wall_us),
            Metric::higher_better("MiB/s", self.state_mib_s),
        ]
    }
}

fn render_table(rows: &[Row]) {
    println!();
    let mut table = Table::new([
        "stage / scale",
        "iters",
        "allocs/op",
        "bytes/op",
        "wall µs",
        "user µs/op",
        "sys µs/op",
        "CPU%",
        "state MiB/s",
    ])
    .aligns([Align::Left]);
    for r in rows {
        table.row([
            r.label.clone(),
            r.iters.to_string(),
            format!("{:.2}", r.allocs_per_op),
            format_bytes(r.bytes_per_op),
            format!("{:.2}", r.wall_us),
            format!("{:.2}", r.user_us),
            format!("{:.2}", r.sys_us),
            format!("{:.0}%", r.cpu_pct),
            format!("{:.1}", r.state_mib_s),
        ]);
    }
    table.print();
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

    let protocol = build_protocol();
    let mut rows: Vec<Row> = Vec::new();

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
        let m = bench(budget, state_bytes, 50, 200_000, || {
            std::hint::black_box(run_parse(&state_json));
        });
        rows.push(Row::from_measurement(&format!("parse/{scale}"), m));

        // render (pre-parsed)
        let m = bench(budget, state_bytes, 50, 200_000, || {
            std::hint::black_box(run_render(&protocol, &state, output_cap));
        });
        rows.push(Row::from_measurement(&format!("render/{scale}"), m));

        // parse + render (the real per-request cost)
        let m = bench(budget, state_bytes, 50, 200_000, || {
            std::hint::black_box(run_parse_render(&protocol, &state_json, output_cap));
        });
        rows.push(Row::from_measurement(&format!("parse+render/{scale}"), m));
    }

    render_table(&rows);

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
        Mode::Save(name) => baseline::save(KIND, &name, SCHEMA, rows),
        Mode::Compare(name) => {
            let _ = baseline::compare(KIND, &name, SCHEMA, &rows, 5.0);
        }
    }
}
