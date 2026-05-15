# WebUI Benchmark Suite

WebUI ships a layered benchmark suite for measuring SSR rendering
performance. Each layer answers a different question, so a thorough
performance investigation runs **multiple** benches before & after a
change and compares.

This document is the reference for what to run, when to run it, and
how to compare results.

> **This commit** adds the `StreamingWriter` / `ChunkPool` primitive
> plus three new bench layers on top of the baseline-only benches
> from the previous commit. The full bench matrix at this commit
> covers `string` / `string+postinject` (legacy paths) and
> `streaming` / `streaming POOLED` (the new primitive). The next
> commit adds the signal-based per-render injection API and the
> corresponding `streaming+inject(opts)` rows.

## Quick reference

| Bench | Layer | Wall time | What it measures | Use when |
|---|---|---|---|---|
| `cargo xtask bench all` | criterion micro | ~5 min | per-fn wall-clock for parser, handler, protocol, expressions, state, webui (incl. streaming + contact-book) | full snapshot of every micro-bench |
| `cargo xtask bench streaming` | criterion micro | ~60 s | writer-path wall-clock + first-chunk TTFB | inner-loop iteration on the streaming module |
| `cargo xtask bench contact-book` | criterion micro | ~90 s | end-to-end render at 10/100/1000 contacts | inner-loop iteration on handler/state/expressions |
| `cargo xtask bench streaming-resource` | example | ~30 s | exact alloc count + bytes + getrusage CPU + RSS | proving zero-alloc claims; allocation regression hunting |
| `cargo xtask bench streaming-e2e-ttfb` | example | ~10 s | HTTP-level TTFB / TTLB through actix | confirming wire-level streaming win |
| `cargo xtask bench streaming-browser` | Playwright | ~30 s | real Chromium TTFB / FCP / LCP / DCL / load | proving user-perceived paint improvement |
| `cargo xtask bench full` (= `streaming-all`) | suite | ~3 min | runs all four streaming-related benches in sequence | full streaming evidence pack for a PR |

## The before/after workflow

All benches support **named baselines**. The flag pattern is
identical across criterion, example, and Playwright benches:

```bash
# 1. Snapshot current numbers as 'before'
cargo xtask bench full --save-baseline before

# 2. Make your change …

# 3. Compare against 'before'
cargo xtask bench full --baseline before
```

Baselines are stored at `target/bench-baselines/`:

* `resource-<name>.json`            — alloc + RSS + CPU table
* `e2e-ttfb-<name>.json`            — HTTP TTFB/TTLB table
* `browser-<name>.json`             — browser metrics table
* `target/criterion/<bench>/<name>` — criterion's native baseline
                                       directory tree

The compare phase prints a Δ%-table for every row. Negative Δ% =
improvement; positive = regression.

### Threshold guidance

| Source | Treat as noise | Treat as signal |
|---|---|---|
| criterion (well-isolated wall-clock) | < ±2% | > ±5% |
| streaming-resource (alloc count) | exact — any change matters | any non-zero |
| streaming-resource (bytes, CPU) | < ±2% | > ±5% |
| streaming-e2e-ttfb (loopback) | < ±10% | > ±20% |
| streaming-browser (real Chromium) | < ±5% | > ±15% |

## Anatomy of each bench

### Criterion benches (`cargo bench`-driven)

Standard criterion harnesses. Each crate has its own `benches/` dir:

* `crates/webui-parser/benches/parser_bench.rs`
* `crates/webui-protocol/benches/protocol_bench.rs`
* `crates/webui-handler/benches/handler_bench.rs`
* `crates/webui-expressions/benches/expressions_bench.rs`
* `crates/webui-state/benches/state_bench.rs`
* `crates/webui/benches/contact_book_bench.rs` — end-to-end render
* `crates/webui/benches/streaming_bench.rs` — writer-path wall-clock + TTFB

These integrate with criterion's HTML reports
(`target/criterion/report/index.html`) and native baseline support
(`--save-baseline NAME` / `--baseline NAME`). `cargo xtask bench`
passes those flags through so you don't need to remember `cargo
bench` invocation details.

### `streaming-resource` (counting allocator + getrusage)

`crates/webui/examples/streaming_resource_bench.rs`

A standalone example binary that installs a custom `GlobalAlloc`
counting every `alloc`/`alloc_zeroed`/growing `realloc` call, then
runs each render path 2000 times and prints a table with:

* **allocs/run** — exact (every `alloc` is counted).
* **bytes/run** — exact total bytes requested.
* **wall µs** — `Instant::now()` per-iteration average.
* **user µs/run** — `getrusage(RUSAGE_SELF).ru_utime` delta / iters.
* **sys µs/run** — `ru_stime` delta / iters.
* **process RSS** — `ru_maxrss` high-water mark at phase end.

Baseline support uses a JSON snapshot format compatible with
`--save NAME` / `--compare NAME` (also wired into `cargo xtask bench
streaming-resource --save-baseline NAME` / `--baseline NAME`).

### `streaming-e2e-ttfb` (in-process actix)

`crates/webui/examples/streaming_e2e_ttfb_bench.rs`

Boots a real actix-web server in a background thread, then makes
HTTP GETs against `/buf` (buffered) and `/stream` (streaming)
endpoints. Measures `responseStart - requestStart` (TTFB) and
`responseEnd - requestStart` (TTLB) using a synthetic per-write
delay (`?delay_us=`) to simulate slower-rendering pages. Reports
median + p99 across N iterations per scenario.

### `streaming-browser` (Playwright in real Chromium)

`examples/integration/streaming-browser-bench/`

The most realistic bench: a Playwright suite that boots a small
hand-built Rust server with `/buf` and `/stream` endpoints, then
navigates a real Chromium tab to each and reports browser-perceived
metrics from `PerformanceObserver`:

* **TTFB** — `responseStart - requestStart`
* **FCP** — first-contentful-paint
* **LCP** — largest-contentful-paint
* **DCL** — DOMContentLoaded
* **load** — load event

The server is intentionally hand-built (does not use the WebUI
handler) so the bench isolates the streaming-vs-buffered question
without confounding from handler implementation details. Baseline
support via `WEBUI_BENCH_SAVE` / `WEBUI_BENCH_COMPARE` env vars,
which `cargo xtask bench streaming-browser --save-baseline NAME` /
`--baseline NAME` set automatically.

## Coming in the next commit

* **`streaming+inject(opts)` rows** — once the structural
  signal-based injection API (`RenderOptions::with_head_inject` /
  `with_body_inject`) lands, both the criterion bench and the
  resource bench gain rows measuring the new inject path against
  the legacy `string+postinject` baseline.
