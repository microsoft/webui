// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! End-to-end benchmark for the contact-book-manager example application.
//!
//! Compiles the real contact-book-manager templates into a protocol binary at
//! benchmark time, then measures protocol parsing and handler rendering at
//! different data scales (10 / 100 / 1,000 contacts).
//!
//! Run with: `cargo bench -p webui --bench contact_book_bench`

use criterion::{criterion_group, BenchmarkId, Criterion, Throughput};
use serde_json::{json, Value};
use std::hint::black_box;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use webui::{build, BuildOptions, CssStrategy, ResponseWriter, WebUIHandler};
use webui_handler::plugin::fast::FastHydrationPlugin;
use webui_handler::RenderOptions;
use webui_protocol::WebUIProtocol;

/// Contact counts to benchmark.
const CONTACT_COUNTS: &[usize] = &[10, 100, 1000];

/// Measurement time per benchmark — 30 s gives criterion enough samples for
/// stable statistics even at the 1,000-contact scale.
const MEASUREMENT_TIME: Duration = Duration::from_secs(30);

/// Minimum time to spend collecting samples for the summary table.
const SUMMARY_MIN_TIME: Duration = Duration::from_secs(3);

/// Estimated bytes of HTML output per contact (without plugin).
/// Used to pre-allocate the writer buffer: `count * BYTES_PER_CONTACT + BASE_HTML_BYTES`.
const BYTES_PER_CONTACT: usize = 512;

/// Estimated bytes of HTML output per contact with the hydration plugin.
/// Plugin adds binding markers per signal/loop, increasing output ~50%.
const BYTES_PER_CONTACT_WITH_PLUGIN: usize = 768;

/// Approximate size of the static HTML shell (head, header, sidebar, chrome)
/// that is independent of the contact count.
const BASE_HTML_BYTES: usize = 8_192;

/// Same as [`BASE_HTML_BYTES`] but for plugin output which includes extra
/// hydration scaffolding in the shell.
const BASE_HTML_BYTES_WITH_PLUGIN: usize = 16_384;

/// Extra headroom added to the bench writer beyond the warmup-measured size,
/// so the buffer is never reallocated during timed iterations.
const WRITER_HEADROOM: usize = 1_024;

/// Initial capacity for the summary writer. 64 KiB is enough for the largest
/// render (1,000 contacts with plugin produces ~443 KiB, but the summary pass
/// reuses the same writer via `clear()`).
const SUMMARY_WRITER_CAPACITY: usize = 64 * 1024;

/// Expected upper bound of samples collected per summary scenario.
/// Used only for `Vec::with_capacity` — does not limit collection.
const EXPECTED_SUMMARY_SAMPLES: usize = 500;

/// Number of warmup iterations before collecting parse samples.
const PARSE_WARMUP_ITERATIONS: usize = 10;

/// Number of warmup iterations before collecting render samples.
const RENDER_WARMUP_ITERATIONS: usize = 3;

/// Maximum number of recent contacts shown on the dashboard page.
const MAX_RECENT_CONTACTS: usize = 5;

// ---------------------------------------------------------------------------
// Bench writer — captures rendered HTML into a pre-allocated buffer
// ---------------------------------------------------------------------------

struct BenchWriter {
    output: String,
}

impl BenchWriter {
    fn new(capacity: usize) -> Self {
        Self {
            output: String::with_capacity(capacity),
        }
    }

    fn clear(&mut self) {
        self.output.clear();
    }

    fn len(&self) -> usize {
        self.output.len()
    }
}

impl ResponseWriter for BenchWriter {
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        self.output.push_str(content);
        Ok(())
    }

    fn end(&mut self) -> webui_handler::Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Realistic contact state generators
// ---------------------------------------------------------------------------

const FIRST_NAMES: &[&str] = &[
    "Sarah", "Marcus", "Yuki", "Priya", "James", "Amara", "Luis", "Emma", "David", "Fatima",
    "Carlos", "Aisha", "Ryan", "Sofia", "Kenji", "Olivia", "Mateo", "Ava", "Noah", "Mia",
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
    "Mendoza",
    "Patel",
    "Mitchell",
    "Andersson",
    "Watanabe",
    "Garcia",
    "Smith",
    "Williams",
    "Brown",
    "Jones",
];

const COMPANIES: &[&str] = &[
    "Contoso Ltd",
    "Fabrikam Inc",
    "Northwind Traders",
    "Adventure Works",
    "Ramirez Photography",
    "Patel & Associates",
    "Watanabe Martial Arts",
    "",
];

const GROUPS: &[&str] = &["Family", "Work", "Friends", "Other"];

const AVATAR_COLORS: &[&str] = &[
    "#4A90D9", "#E67E22", "#2ECC71", "#9B59B6", "#3498DB", "#E74C3C", "#1ABC9C", "#F39C12",
    "#8E44AD", "#16A085", "#D35400", "#2980B9", "#27AE60", "#C0392B", "#7F8C8D",
];

const CITIES: &[&str] = &[
    "Seattle, WA 98101",
    "Portland, OR 97201",
    "San Francisco, CA 94103",
    "Austin, TX 78701",
    "Boston, MA 02108",
    "Atlanta, GA 30301",
    "Miami, FL 33101",
    "New York, NY 10007",
    "Chicago, IL 60601",
    "Denver, CO 80201",
];

fn generate_contact(idx: usize) -> Value {
    let first = FIRST_NAMES[idx % FIRST_NAMES.len()];
    let last = LAST_NAMES[idx % LAST_NAMES.len()];
    let initials = format!("{}{}", &first[..1], &last.chars().next().unwrap_or('?'));
    let company = COMPANIES[idx % COMPANIES.len()];
    let group = GROUPS[idx % GROUPS.len()];
    let color = AVATAR_COLORS[idx % AVATAR_COLORS.len()];
    let city = CITIES[idx % CITIES.len()];
    let favorite = idx % 3 == 0;

    json!({
        "id": (idx + 1).to_string(),
        "firstName": first,
        "lastName": last,
        "email": format!("{}.{}@example.com", first.to_lowercase(), last.to_lowercase()),
        "phone": format!("+1 (555) {:03}-{:04}", (idx * 111) % 1000, (idx * 1234) % 10000),
        "company": company,
        "group": group,
        "favorite": favorite,
        "initials": initials,
        "avatarColor": color,
        "notes": if idx % 2 == 0 { format!("Contact note for {} {}", first, last) } else { String::new() },
        "address": format!("{} {} St, {}", (idx + 1) * 100, first, city),
    })
}

fn build_contact_state(contact_count: usize) -> Value {
    let contacts: Vec<Value> = (0..contact_count).map(generate_contact).collect();

    let favorites: Vec<Value> = contacts
        .iter()
        .filter(|c| c["favorite"] == json!(true))
        .cloned()
        .collect();
    let favorite_count = favorites.len();

    let recent_count = std::cmp::min(MAX_RECENT_CONTACTS, contact_count);
    let recent: Vec<Value> = contacts[contact_count.saturating_sub(recent_count)..].to_vec();

    json!({
        "page": "dashboard",
        "searchQuery": "",
        "activeGroup": "all",
        "groups": GROUPS,
        "totalContacts": contact_count,
        "totalFavorites": favorite_count,
        "totalGroups": GROUPS.len(),
        "contacts": contacts,
        "filteredContacts": contacts,
        "recentContacts": recent,
        "favoriteContacts": favorites,
        "selectedContact": null
    })
}

// ---------------------------------------------------------------------------
// Build protocol from the contact-book-manager example app
// ---------------------------------------------------------------------------

fn contact_book_app_dir() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("..")
        .join("..")
        .join("examples")
        .join("app")
        .join("contact-book-manager")
        .join("src")
}

/// Builds the protocol binary from the live contact-book-manager source.
///
/// This compiles the real templates at benchmark time — there is no cached
/// `.pb` file. Any change to `examples/app/contact-book-manager/src/` is
/// automatically reflected in the next benchmark run, making regressions and
/// improvements traceable to specific template changes.
fn build_contact_book_protocol() -> (WebUIProtocol, Vec<u8>) {
    let app_dir = contact_book_app_dir();
    assert!(
        app_dir.join("index.html").exists(),
        "contact-book-manager source not found at {}",
        app_dir.display()
    );

    let result = build(BuildOptions {
        app_dir,
        entry: "index.html".to_string(),
        css: CssStrategy::Style,
        plugin: None,
        components: Vec::new(),
    })
    .expect("failed to build contact-book-manager protocol");

    let bytes = result.protocol_bytes.clone();
    (result.protocol, bytes)
}

// ---------------------------------------------------------------------------
// One-time setup: build protocol + generate all state sizes up-front
// ---------------------------------------------------------------------------

struct BenchFixture {
    protocol: WebUIProtocol,
    protocol_bytes: Vec<u8>,
    states: Vec<(usize, Value)>,
}

fn setup() -> BenchFixture {
    let (protocol, protocol_bytes) = build_contact_book_protocol();
    let states = CONTACT_COUNTS
        .iter()
        .map(|&n| (n, build_contact_state(n)))
        .collect();
    BenchFixture {
        protocol,
        protocol_bytes,
        states,
    }
}

// ---------------------------------------------------------------------------
// Benchmark: Protocol deserialization
// ---------------------------------------------------------------------------

fn protocol_deserialization_bench(c: &mut Criterion) {
    let fixture = setup();
    let mut group = c.benchmark_group("contact_book_protocol_parse");
    group.measurement_time(MEASUREMENT_TIME);

    group.throughput(Throughput::Bytes(fixture.protocol_bytes.len() as u64));

    group.bench_function("from_protobuf", |b| {
        b.iter(|| {
            let protocol = WebUIProtocol::from_protobuf(black_box(&fixture.protocol_bytes))
                .expect("protocol deserialization failed");
            black_box(&protocol);
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Benchmark: Handler rendering (no plugin) at different contact counts
// ---------------------------------------------------------------------------

fn handler_rendering_bench(c: &mut Criterion) {
    let fixture = setup();
    let mut group = c.benchmark_group("contact_book_render");
    group.measurement_time(MEASUREMENT_TIME);

    for (count, state) in &fixture.states {
        // Pre-render to measure output size for throughput calculation
        let handler = WebUIHandler::new();
        let mut warmup_writer = BenchWriter::new(count * BYTES_PER_CONTACT + BASE_HTML_BYTES);
        handler
            .handle(
                &fixture.protocol,
                state,
                &RenderOptions::new("index.html", "/"),
                &mut warmup_writer,
            )
            .unwrap_or_else(|e| panic!("warmup render failed for {count} contacts: {e}"));
        group.throughput(Throughput::Bytes(warmup_writer.len() as u64));

        group.bench_with_input(BenchmarkId::new("contacts", count), state, |b, state| {
            let h = WebUIHandler::new();
            let mut w = BenchWriter::new(warmup_writer.len() + WRITER_HEADROOM);

            b.iter(|| {
                w.clear();
                h.handle(
                    black_box(&fixture.protocol),
                    black_box(state),
                    &RenderOptions::new("index.html", "/"),
                    &mut w,
                )
                .unwrap_or_else(|e| panic!("render failed for {count} contacts: {e}"));
            });
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Benchmark: Handler rendering with FastHydration plugin
// ---------------------------------------------------------------------------

fn handler_rendering_with_plugin_bench(c: &mut Criterion) {
    let fixture = setup();
    let mut group = c.benchmark_group("contact_book_render_fast_plugin");
    group.measurement_time(MEASUREMENT_TIME);

    for (count, state) in &fixture.states {
        let handler = WebUIHandler::with_plugin(|| Box::new(FastHydrationPlugin::new()));
        let mut warmup_writer =
            BenchWriter::new(count * BYTES_PER_CONTACT_WITH_PLUGIN + BASE_HTML_BYTES_WITH_PLUGIN);
        handler
            .handle(
                &fixture.protocol,
                state,
                &RenderOptions::new("index.html", "/"),
                &mut warmup_writer,
            )
            .unwrap_or_else(|e| {
                panic!("warmup render with plugin failed for {count} contacts: {e}")
            });
        group.throughput(Throughput::Bytes(warmup_writer.len() as u64));

        group.bench_with_input(BenchmarkId::new("contacts", count), state, |b, state| {
            let h = WebUIHandler::with_plugin(|| Box::new(FastHydrationPlugin::new()));
            let mut w = BenchWriter::new(warmup_writer.len() + WRITER_HEADROOM);

            b.iter(|| {
                w.clear();
                h.handle(
                    black_box(&fixture.protocol),
                    black_box(state),
                    &RenderOptions::new("index.html", "/"),
                    &mut w,
                )
                .unwrap_or_else(|e| panic!("render with plugin failed for {count} contacts: {e}"));
            });
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Criterion harness
// ---------------------------------------------------------------------------

criterion_group!(
    benches,
    protocol_deserialization_bench,
    handler_rendering_bench,
    handler_rendering_with_plugin_bench,
);

// ---------------------------------------------------------------------------
// Summary table — performance report printed after criterion finishes
// ---------------------------------------------------------------------------

struct SummaryRow {
    name: String,
    iterations: usize,
    samples_ms: Vec<f64>,
    output_bytes: usize,
}

impl SummaryRow {
    fn avg(&self) -> f64 {
        self.samples_ms.iter().sum::<f64>() / self.samples_ms.len() as f64
    }

    fn min(&self) -> f64 {
        self.samples_ms
            .iter()
            .cloned()
            .reduce(f64::min)
            .unwrap_or(0.0)
    }

    fn max(&self) -> f64 {
        self.samples_ms
            .iter()
            .cloned()
            .reduce(f64::max)
            .unwrap_or(0.0)
    }

    fn percentile(&self, p: f64) -> f64 {
        if self.samples_ms.is_empty() {
            return 0.0;
        }
        let mut sorted = self.samples_ms.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let idx = (p / 100.0 * (sorted.len() - 1) as f64).round() as usize;
        sorted[idx.min(sorted.len() - 1)]
    }

    fn iqr(&self) -> f64 {
        self.percentile(75.0) - self.percentile(25.0)
    }

    fn std_dev(&self) -> f64 {
        let avg = self.avg();
        let variance = self
            .samples_ms
            .iter()
            .map(|s| (s - avg).powi(2))
            .sum::<f64>()
            / self.samples_ms.len() as f64;
        variance.sqrt()
    }

    fn dev_pct(&self) -> f64 {
        let avg = self.avg();
        if avg == 0.0 {
            return 0.0;
        }
        (self.std_dev() / avg) * 100.0
    }
}

fn collect_parse_samples(name: &str, bytes: &[u8]) -> SummaryRow {
    for _ in 0..PARSE_WARMUP_ITERATIONS {
        let _ = WebUIProtocol::from_protobuf(black_box(bytes));
    }

    let mut samples = Vec::with_capacity(EXPECTED_SUMMARY_SAMPLES);
    let deadline = Instant::now() + SUMMARY_MIN_TIME;

    while Instant::now() < deadline {
        let start = Instant::now();
        let protocol =
            WebUIProtocol::from_protobuf(black_box(bytes)).expect("parse failed in summary pass");
        black_box(&protocol);
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
    }

    SummaryRow {
        name: name.to_string(),
        iterations: samples.len(),
        samples_ms: samples,
        output_bytes: bytes.len(),
    }
}

fn collect_render_samples(
    name: &str,
    protocol: &WebUIProtocol,
    state: &Value,
    use_plugin: bool,
) -> SummaryRow {
    // Warm up — also measures output size
    let handler = if use_plugin {
        WebUIHandler::with_plugin(|| Box::new(FastHydrationPlugin::new()))
    } else {
        WebUIHandler::new()
    };
    let mut writer = BenchWriter::new(SUMMARY_WRITER_CAPACITY);
    for _ in 0..RENDER_WARMUP_ITERATIONS {
        writer.clear();
        handler
            .handle(
                protocol,
                state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .expect("warmup failed in summary pass");
    }
    let output_bytes = writer.len();

    // Collect timing samples
    let mut samples = Vec::with_capacity(EXPECTED_SUMMARY_SAMPLES);
    let deadline = Instant::now() + SUMMARY_MIN_TIME;

    while Instant::now() < deadline {
        writer.clear();
        let start = Instant::now();
        handler
            .handle(
                black_box(protocol),
                black_box(state),
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .expect("render failed in summary pass");
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
    }

    SummaryRow {
        name: name.to_string(),
        iterations: samples.len(),
        samples_ms: samples,
        output_bytes,
    }
}

fn print_summary(rows: &[SummaryRow]) {
    let width = 100;
    let border = "=".repeat(width);
    let dash = "-".repeat(width);

    eprintln!();
    eprintln!(
        "{:=^width$}",
        " WebUI Contact Book — Performance Summary ",
        width = width
    );
    eprintln!(
        "{:<24} {:>6} {:>9} {:>9} {:>9} {:>6} {:>9} {:>9} {:>9} {:>9} {:>9}",
        "Story", "Iters", "Avg(ms)", "Min", "Max", "Dev%", "P50", "P90", "P99", "IQR", "Bytes"
    );
    eprintln!("{dash}");

    for row in rows {
        eprintln!(
            "{:<24} {:>6} {:>9.2} {:>9.2} {:>9.2} {:>5.1}% {:>9.2} {:>9.2} {:>9.2} {:>9.2} {:>9}",
            row.name,
            row.iterations,
            row.avg(),
            row.min(),
            row.max(),
            row.dev_pct(),
            row.percentile(50.0),
            row.percentile(90.0),
            row.percentile(99.0),
            row.iqr(),
            row.output_bytes,
        );
    }

    eprintln!("{border}");
    eprintln!("IQR = P75 − P25 (lower means more consistent)");
    eprintln!("All times in milliseconds");
    eprintln!("{border}");
}

fn run_summary_pass(fixture: &BenchFixture) {
    let mut rows = Vec::new();

    rows.push(collect_parse_samples(
        "ProtocolParse",
        &fixture.protocol_bytes,
    ));

    for (count, state) in &fixture.states {
        rows.push(collect_render_samples(
            &format!("Render/{count}"),
            &fixture.protocol,
            state,
            false,
        ));
    }

    for (count, state) in &fixture.states {
        rows.push(collect_render_samples(
            &format!("RenderFAST/{count}"),
            &fixture.protocol,
            state,
            true,
        ));
    }

    print_summary(&rows);
}

// ---------------------------------------------------------------------------
// Custom main — runs criterion then prints the summary table
// ---------------------------------------------------------------------------

fn main() {
    // Run criterion benchmarks (handles --test, filters, etc.)
    benches();

    // Print summary table unless running in --test mode
    let is_test_mode = std::env::args().any(|a| a == "--test");
    if !is_test_mode {
        let fixture = setup();
        run_summary_pass(&fixture);
    }
}
