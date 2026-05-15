# WebUI Benchmark Suite

WebUI ships a layered benchmark suite for measuring SSR rendering
performance. Each layer answers a different question, so a thorough
performance investigation runs **multiple** benches before & after a
change and compares.

This document is the reference for what to run, when to run it, and
how to compare results.

> **This commit** is the first in a multi-commit pipeline that adds
> the streaming SSR feature. At this commit, only the *baseline*
> render paths exist: `string` (pre-allocated buffer) and
> `string+postinject` (legacy buffer-then-byte-scan injection).
> Subsequent commits add the `streaming` writer, the
> `streaming+inject(opts)` signal-based injection, an end-to-end TTFB
> bench, and the real-Chromium Playwright bench — all measurable
> against the baselines captured here.

## Quick reference

| Bench | Layer | Wall time | What it measures | Use when |
|---|---|---|---|---|
| `cargo xtask bench all` | criterion micro | ~5 min | per-fn wall-clock for parser, handler, protocol, expressions, state, webui | full snapshot of every micro-bench |
| `cargo xtask bench streaming` | criterion micro | ~60 s | writer-path wall-clock (`string`, `string+postinject` at this commit) | inner-loop iteration on the rendering module |
| `cargo xtask bench contact-book` | criterion micro | ~90 s | end-to-end render at 10/100/1000 contacts | inner-loop iteration on handler/state/expressions |
| `cargo xtask bench streaming-resource` | example | ~30 s | exact alloc count + bytes + getrusage CPU + RSS | proving zero-alloc claims; allocation regression hunting |
| `cargo xtask bench full` (= `streaming-all`) | suite | ~2 min | runs criterion writer-paths + resource bench in sequence | quick before/after snapshot |

## The before/after workflow

All benches support **named baselines**. The flag pattern is
identical across criterion and example benches:

```bash
# 1. Snapshot current numbers as 'before'
cargo xtask bench full --save-baseline before

# 2. Make your change …

# 3. Compare against 'before'
cargo xtask bench full --baseline before
```

Baselines are stored at `target/bench-baselines/`:

* `resource-<name>.json`        — alloc + RSS + CPU table
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

## Anatomy of each bench

### Criterion benches (`cargo bench`-driven)

Standard criterion harnesses. Each crate has its own `benches/` dir:

* `crates/webui-parser/benches/parser_bench.rs`
* `crates/webui-protocol/benches/protocol_bench.rs`
* `crates/webui-handler/benches/handler_bench.rs`
* `crates/webui-expressions/benches/expressions_bench.rs`
* `crates/webui-state/benches/state_bench.rs`
* `crates/webui/benches/contact_book_bench.rs` — end-to-end render
* `crates/webui/benches/streaming_bench.rs` — writer-path wall-clock

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

The baseline support uses the same JSON snapshot format as the other
non-criterion benches, so before/after deltas show up as a Δ%-table.

```bash
cargo xtask bench streaming-resource --save-baseline before
# … change …
cargo xtask bench streaming-resource --baseline before
```

## Coming in later commits

* **`streaming` writer-path row** — once `StreamingWriter` lands, the
  criterion `writer_paths` group and the resource bench gain a
  streaming row that can be diffed against the `string` baseline
  captured here.
* **`streaming+inject(opts)` row** — once the structural signal-based
  injection API lands, both benches gain a row measuring the new
  inject path against the legacy `string+postinject` baseline.
* **`streaming-e2e-ttfb`** — in-process actix server measuring real
  HTTP TTFB / TTLB.
* **`streaming-browser`** — Playwright in real Chromium measuring
  TTFB / FCP / LCP / DCL / load.

The full reference for those benches lands in the commit that
introduces each one.
