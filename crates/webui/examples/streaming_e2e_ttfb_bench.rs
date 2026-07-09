// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! End-to-end HTTP-level TTFB benchmark for the streaming render path.
//!
//! Spawns a real actix-web server with two endpoints:
//!
//! * `/buf`    — renders the contact-book protocol into a `String`,
//!               returns the whole body in one HTTP response chunk.
//!               Mirrors what `pnpm start:server` did before streaming.
//! * `/stream` — renders into the streaming pipeline (`StreamingWriter`
//!               + bounded mpsc + `ReceiverStream`), exactly as the
//!               production `webui-cli` and commerce server do.
//!
//! Both endpoints accept a `delay_us` query parameter that injects a
//! per-`write()` artificial delay on the producer side. This simulates
//! a slower render (e.g., a real e-commerce page that takes 5–20 ms
//! to produce) so we can measure the streaming TTFB win at realistic
//! scales — not just the trivial 35 µs render we have in the contact-
//! book bench.
//!
//! Measurements (using `awc` as the HTTP client):
//!
//! * **TTFB** — milliseconds from request send to first response byte
//! * **TTLB** — milliseconds from request send to last response byte
//! * **delta** — TTLB − TTFB (how much "extra" the streaming path
//!                buys for the parser/browser to start work early)
//!
//! Run with:
//!
//! ```sh
//! cargo run --release --example streaming_e2e_ttfb_bench -p microsoft-webui
//! ```
//!
//! ## Why TTFB ≠ FCP / LCP / TTI
//!
//! This benchmark measures **HTTP-level** TTFB: when the first byte
//! arrives at an HTTP client. It does NOT measure browser-perceived
//! metrics like First Contentful Paint, Largest Contentful Paint, or
//! Time to Interactive — those depend on parser progress, CSS
//! cascade, JS execution, and font loading, all of which require a
//! real browser harness (Playwright with `PerformanceObserver`).
//!
//! The HTTP-level TTFB win is a **necessary but not sufficient**
//! condition for browser-level paint wins. If TTFB doesn't drop here,
//! FCP/LCP can't possibly improve. If TTFB does drop, browser-level
//! benefit depends on whether the early bytes contain enough
//! head/CSS for the browser to start parsing/rendering — usually true
//! for SSR HTML.

#![allow(missing_docs)]

use actix_web::{web, App, HttpResponse, HttpServer};
use awc::Client;
use bytes::Bytes;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use webui::streaming::StreamingWriter;
use webui::{build, BuildOptions, CssStrategy, ResponseWriter, WebUIHandler};
use webui_bench_support::report::{Align, Table};
use webui_bench_support::{baseline, percentile, BaselineRow, Metric};
use webui_handler::RenderOptions;
use webui_protocol::WebUIProtocol;

// ── Shared protocol & state ────────────────────────────────────────────

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

// ── Server state shared across handlers ────────────────────────────────

struct ServerState {
    protocol: WebUIProtocol,
    state: Value,
}

#[derive(Deserialize)]
struct DelayQuery {
    /// Per-`write()` artificial delay in microseconds. 0 = instant.
    /// Use small positive values to simulate large/slow renders.
    /// Total render delay ≈ `delay_us * write_count` (write_count for
    /// the contact-book template is ~525).
    delay_us: Option<u64>,
}

// ── /buf — buffered render path ────────────────────────────────────────

/// `ResponseWriter` that buffers into a `String` AND optionally sleeps
/// before each write to simulate a slower render.
struct DelayingStringWriter {
    buf: String,
    delay: Duration,
}
impl DelayingStringWriter {
    fn new(cap: usize, delay: Duration) -> Self {
        Self {
            buf: String::with_capacity(cap),
            delay,
        }
    }
}
impl ResponseWriter for DelayingStringWriter {
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        if !self.delay.is_zero() {
            std::thread::sleep(self.delay);
        }
        self.buf.push_str(content);
        Ok(())
    }
    fn end(&mut self) -> webui_handler::Result<()> {
        Ok(())
    }
}

async fn handle_buf(
    state: web::Data<Arc<ServerState>>,
    query: web::Query<DelayQuery>,
) -> HttpResponse {
    let delay = Duration::from_micros(query.delay_us.unwrap_or(0));
    let st = state.clone();
    // Run the render on a blocking worker so we don't park the runtime.
    let html = actix_web::rt::task::spawn_blocking(move || {
        let h = WebUIHandler::new();
        let mut w = DelayingStringWriter::new(64 * 1024, delay);
        h.handle(
            &st.protocol,
            &st.state,
            &RenderOptions::new("index.html", "/"),
            &mut w,
        )
        .expect("render");
        w.buf
    })
    .await
    .expect("join");
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(html)
}

// ── /stream — streaming render path ────────────────────────────────────

/// Wraps `StreamingWriter` with the same delay injection so both
/// endpoints have identical render-time characteristics; only the
/// delivery mechanism differs.
struct DelayingStreamingWriter {
    inner: StreamingWriter,
    delay: Duration,
}
impl ResponseWriter for DelayingStreamingWriter {
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        if !self.delay.is_zero() {
            std::thread::sleep(self.delay);
        }
        self.inner.write(content)
    }
    fn end(&mut self) -> webui_handler::Result<()> {
        self.inner.end()
    }
}

async fn handle_stream(
    state: web::Data<Arc<ServerState>>,
    query: web::Query<DelayQuery>,
) -> HttpResponse {
    let delay = Duration::from_micros(query.delay_us.unwrap_or(0));
    let st = state.clone();
    let (tx, rx) = mpsc::channel::<Bytes>(StreamingWriter::DEFAULT_CHANNEL_CAPACITY);
    actix_web::rt::task::spawn_blocking(move || {
        let inner = StreamingWriter::new(tx);
        let mut writer = DelayingStreamingWriter { inner, delay };
        let h = WebUIHandler::new();
        // RenderOptions inject — handler emits at the structural
        // head_end/body_end signal boundaries; zero scan cost.
        let opts = RenderOptions::new("index.html", "/")
            .with_head_inject("<link rel=preload>")
            .with_body_inject("<script>/* lr */</script>");
        let _ = h.handle(&st.protocol, &st.state, &opts, &mut writer);
        let _ = ResponseWriter::end(&mut writer);
    });
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx).map(Ok::<Bytes, actix_web::Error>);
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .streaming(stream)
}

// ── Server boot ────────────────────────────────────────────────────────

fn start_server() -> u16 {
    let protocol = build_protocol();
    let state = build_state(100);
    let shared = Arc::new(ServerState { protocol, state });

    let (port_tx, port_rx) = std::sync::mpsc::channel::<u16>();
    thread::spawn(move || {
        let sys = actix_web::rt::System::new();
        sys.block_on(async move {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
            let port = listener.local_addr().expect("addr").port();
            port_tx.send(port).expect("port tx");
            let data = web::Data::new(shared);
            HttpServer::new(move || {
                App::new()
                    .app_data(data.clone())
                    .route("/buf", web::get().to(handle_buf))
                    .route("/stream", web::get().to(handle_stream))
            })
            .listen(listener)
            .expect("listen")
            .workers(2)
            .run()
            .await
            .expect("run");
        });
    });
    port_rx.recv().expect("server port")
}

// ── HTTP client measurements ───────────────────────────────────────────

#[derive(Default, Clone, Copy)]
struct Measurement {
    ttfb_us: u128,
    ttlb_us: u128,
    body_bytes: usize,
}

async fn measure_one(client: &Client, url: &str) -> Measurement {
    let start = Instant::now();
    let mut resp = client.get(url).send().await.expect("send");
    let ttfb = start.elapsed();
    let mut body_bytes = 0usize;
    // Drain the body, but only the first byte's arrival is "TTFB".
    while let Some(chunk) = resp.next().await {
        let chunk = chunk.expect("chunk");
        body_bytes += chunk.len();
    }
    let ttlb = start.elapsed();
    Measurement {
        ttfb_us: ttfb.as_micros(),
        ttlb_us: ttlb.as_micros(),
        body_bytes,
    }
}

async fn run_scenario(
    client: &Client,
    url: &str,
    iters: usize,
) -> (u128, u128, u128, u128, u128, u128, usize) {
    // Warmup: first few requests wake up actix workers, allocator slabs.
    for _ in 0..5 {
        let _ = measure_one(client, url).await;
    }

    let mut ttfb = Vec::with_capacity(iters);
    let mut ttlb = Vec::with_capacity(iters);
    let mut last_body = 0;
    for _ in 0..iters {
        let m = measure_one(client, url).await;
        ttfb.push(m.ttfb_us);
        ttlb.push(m.ttlb_us);
        last_body = m.body_bytes;
    }

    let ttfb_p50 = percentile(&mut ttfb.clone(), 50.0);
    let ttfb_p99 = percentile(&mut ttfb.clone(), 99.0);
    let ttfb_min = *ttfb.iter().min().unwrap_or(&0);
    let ttlb_p50 = percentile(&mut ttlb.clone(), 50.0);
    let ttlb_p99 = percentile(&mut ttlb.clone(), 99.0);
    let ttlb_min = *ttlb.iter().min().unwrap_or(&0);
    (
        ttfb_min, ttfb_p50, ttfb_p99, ttlb_min, ttlb_p50, ttlb_p99, last_body,
    )
}

// ── Snapshot serialization ────────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize)]
struct SnapshotRow {
    label: String,
    iters: usize,
    ttfb_min_us: u128,
    ttfb_p50_us: u128,
    ttfb_p99_us: u128,
    ttlb_min_us: u128,
    ttlb_p50_us: u128,
    ttlb_p99_us: u128,
    body_bytes: usize,
}

const SCHEMA: u32 = 2;
const KIND: &str = "e2e-ttfb";

impl BaselineRow for SnapshotRow {
    fn key(&self) -> String {
        self.label.clone()
    }

    fn metrics(&self) -> Vec<Metric> {
        // Latency percentiles are lower-better. p50 tracks the typical
        // request; p99 guards the tail that dominates user-perceived
        // slowness. `min` is displayed but intentionally NOT gated — it
        // is the single luckiest sample and far too noisy to threshold.
        vec![
            Metric::lower_better("TTFB p50", self.ttfb_p50_us as f64),
            Metric::lower_better("TTFB p99", self.ttfb_p99_us as f64),
            Metric::lower_better("TTLB p50", self.ttlb_p50_us as f64),
            Metric::lower_better("TTLB p99", self.ttlb_p99_us as f64),
        ]
    }
}

fn render_table(rows: &[SnapshotRow]) {
    let mut table = Table::new([
        "scenario / path",
        "iter",
        "TTFB min",
        "TTFB p50",
        "TTFB p99",
        "TTLB min",
        "TTLB p50",
        "TTLB p99",
        "bytes",
    ])
    .aligns([Align::Left]);
    for r in rows {
        table.row([
            r.label.clone(),
            r.iters.to_string(),
            format!("{} µs", r.ttfb_min_us),
            format!("{} µs", r.ttfb_p50_us),
            format!("{} µs", r.ttfb_p99_us),
            format!("{} µs", r.ttlb_min_us),
            format!("{} µs", r.ttlb_p50_us),
            format!("{} µs", r.ttlb_p99_us),
            r.body_bytes.to_string(),
        ]);
    }
    table.print();
}

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
                    "Usage: streaming_e2e_ttfb_bench [--save NAME] [--compare NAME]\n\n\
                     With no args: prints the table.\n\
                     --save NAME: write current results to target/bench-baselines/e2e-ttfb-NAME.json\n\
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

fn main() {
    let mode = parse_args();
    println!("WebUI streaming end-to-end TTFB benchmark");
    println!("=========================================");
    println!(
        "Build: {}",
        if cfg!(debug_assertions) {
            "DEBUG (rebuild --release)"
        } else {
            "release"
        }
    );

    let port = start_server();
    println!("Server listening on 127.0.0.1:{port}");
    // Give actix a beat to fully accept.
    thread::sleep(Duration::from_millis(200));

    let scenarios: &[(u64, &str)] = &[
        (0, "no delay (real render only, ~35 µs)"),
        (10, "10 µs/write → ~5 ms render (typical small SSR)"),
        (50, "50 µs/write → ~26 ms render (medium SSR)"),
        (200, "200 µs/write → ~105 ms render (large e-commerce)"),
    ];

    let iters = 50;
    let rt = actix_web::rt::System::new();
    println!();
    println!(
        "Running {} scenarios × 2 paths × {iters} iters over HTTP loopback…",
        scenarios.len()
    );
    let snapshot_rows: Vec<SnapshotRow> = rt.block_on(async {
        let client = Client::default();
        let mut rows: Vec<SnapshotRow> = Vec::new();

        for &(delay_us, desc) in scenarios {
            for &(label, route) in &[("buffered", "buf"), ("streaming", "stream")] {
                let url = format!("http://127.0.0.1:{port}/{route}?delay_us={delay_us}");
                let row_label = format!("{label} | {desc}");
                // Live progress: a single scenario at 200 µs/write takes
                // seconds, so echo which one is running to stderr.
                eprintln!("  … {row_label}");
                let (mn1, p50_1, p99_1, mn2, p50_2, p99_2, bytes) =
                    run_scenario(&client, &url, iters).await;
                rows.push(SnapshotRow {
                    label: row_label,
                    iters,
                    ttfb_min_us: mn1,
                    ttfb_p50_us: p50_1,
                    ttfb_p99_us: p99_1,
                    ttlb_min_us: mn2,
                    ttlb_p50_us: p50_2,
                    ttlb_p99_us: p99_2,
                    body_bytes: bytes,
                });
            }
        }
        rows
    });

    render_table(&snapshot_rows);
    println!();
    println!("Notes:");
    println!("  * TTFB = time from request send to first response byte.");
    println!("  * TTLB = time from request send to last response byte.");
    println!("  * No network throttling: requests are loopback (~50 µs RTT).");
    println!("    On real WAN (50 ms RTT), add 50 ms to every number — the");
    println!("    streaming TTFB win STAYS the same in absolute µs, but");
    println!("    relative to the fixed 50 ms baseline becomes negligible.");
    println!("  * For browser-perceived metrics (FCP, LCP, TTI), use a");
    println!("    real browser harness (Playwright + PerformanceObserver).");

    match mode {
        Mode::Print => {}
        Mode::Save(name) => baseline::save(KIND, &name, SCHEMA, snapshot_rows),
        Mode::Compare(name) => {
            // Latency is noisier than CPU/alloc counts, so gate at 15%.
            let _ = baseline::compare(KIND, &name, SCHEMA, &snapshot_rows, 15.0);
        }
    }
}
