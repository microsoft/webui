// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Criterion writer-path benchmarks (commit 1: baseline-only).
//!
//! Measures wall-clock render throughput for the two paths that exist
//! on `origin/main`:
//!
//! 1. **`string`**            — pre-allocated `String` buffer. The
//!    baseline most hosts use today.
//! 2. **`string+postinject`** — `string` followed by a case-insensitive
//!    `</body>` byte-window scan + concat. Mirrors the legacy
//!    dev-server livereload pipeline.
//!
//! Subsequent commits in this branch will add a `streaming` row (once
//! the StreamingWriter primitive lands) and a `streaming+inject(opts)`
//! row (once the signal-based injection API lands). Compare with
//! `cargo bench -p microsoft-webui --bench streaming_bench --
//! --save-baseline NAME` and `--baseline NAME`.

#![allow(missing_docs)]

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use serde_json::{json, Value};
use std::hint::black_box;
use std::path::PathBuf;
use std::time::Duration;
use webui::{build, BuildOptions, CssStrategy, DomStrategy, ResponseWriter, WebUIHandler};
use webui_handler::RenderOptions;
use webui_protocol::WebUIProtocol;

const CONTACT_COUNTS: &[usize] = &[10, 100, 1000];
const MEASUREMENT_TIME: Duration = Duration::from_secs(8);
const SAMPLE_SIZE: usize = 50;

// Body inject script used by the `string+postinject` baseline path
// (mirrors the dev-mode livereload script that the legacy `lr.inject`
// post-render pipeline injects). Future commits replace this with
// signal-based injection.
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

// ── Writers ───────────────────────────────────────────────────────────

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

// ── Bench ─────────────────────────────────────────────────────────────

fn bench_writers(c: &mut Criterion) {
    let protocol = build_protocol();
    let states: Vec<(usize, Value)> = CONTACT_COUNTS
        .iter()
        .map(|&n| (n, build_state(n)))
        .collect();

    // Warm-up to compute output size for capacity hints.
    let output_size = {
        let h = WebUIHandler::new();
        let mut w = StringWriter::with_capacity(128 * 1024);
        h.handle(
            &protocol,
            &states[0].1,
            &RenderOptions::new("index.html", "/"),
            &mut w,
        )
        .expect("warmup");
        w.buf.len()
    };

    let mut group = c.benchmark_group("writer_paths");
    group.measurement_time(MEASUREMENT_TIME);
    group.sample_size(SAMPLE_SIZE);

    for (count, state) in &states {
        let count = *count;
        group.throughput(Throughput::Bytes(output_size as u64));

        // Path 1: String (baseline).
        group.bench_with_input(
            BenchmarkId::new(format!("string/{count}"), output_size),
            state,
            |b, state| {
                let h = WebUIHandler::new();
                b.iter(|| {
                    let mut w = StringWriter::with_capacity(output_size);
                    h.handle(
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

        // Path 2: String + post-render injection (mirrors the legacy
        // livereload `lr.inject(&buf)` pipeline).
        group.bench_with_input(
            BenchmarkId::new(format!("string+postinject/{count}"), output_size),
            state,
            |b, state| {
                let h = WebUIHandler::new();
                b.iter(|| {
                    let mut w = StringWriter::with_capacity(output_size);
                    h.handle(
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

criterion_group!(benches, bench_writers);
criterion_main!(benches);
