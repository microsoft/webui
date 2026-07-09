# WebUI Benchmark Suite

WebUI ships a layered benchmark suite for measuring SSR rendering
performance. Each layer answers a different question, so a thorough
performance investigation runs **multiple** benches before & after a
change and compares.

This document is the reference for what to run, when to run it, and
how to compare results.

## Quick reference

| Bench | Layer | Wall time | What it measures | Use when |
|---|---|---|---|---|
| `cargo xtask bench all` | criterion micro | ~5 min | per-fn wall-clock for parser, handler, protocol, expressions, state, webui (incl. streaming + contact-book) | full snapshot of every micro-bench |
| `cargo xtask bench streaming` | criterion micro | ~60 s | writer-path wall-clock + first-chunk TTFB | inner-loop iteration on the streaming module |
| `cargo xtask bench contact-book` | criterion micro | ~90 s | end-to-end render at 10/100/1000 contacts | inner-loop iteration on handler/state/expressions |
| `cargo xtask bench streaming-resource` | example | ~30 s | exact alloc count + bytes + getrusage CPU + RSS | proving zero-alloc claims; allocation regression hunting |
| `cargo xtask bench state-cpu` | example | ~20 s | user/sys CPU µs + **CPU%** + MiB/s for parse vs render vs parse+render at 1k/5k/10k state | attributing per-request CPU to JSON state parsing |
| `cargo xtask bench ffi-cpu` | Node addon | ~40 s (incl. addon build) | full FFI per-request CPU via `process.cpuUsage()` — napi marshalling + parse + render + per-chunk callback | measuring the CPU the Node host actually burns |
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

* `streaming-resource-<name>.json`  — alloc + RSS + CPU table
* `state-cpu-<name>.json`           — parse/render CPU + CPU% table
* `ffi-cpu-<name>.json`             — Node FFI CPU (process.cpuUsage) table
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
| state-cpu (user CPU µs, CPU%) | < ±3% | > ±5% |
| ffi-cpu (user CPU µs) | < ±5% | > ±10% |
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

#### Large-JSON / state-parse coverage

The Node/WASM hosts run `serde_json::from_str::<Value>` on the state
object for **every** render, so that parse — not the template walk —
dominates CPU on list/table-heavy pages with large state. Two groups
target it directly:

* `state_bench.rs → state_parse` — `serde_json::from_str` throughput
  (bytes/s) across `wide_flat_keys` (100–10k top-level keys),
  `catalog_items` (100–10k array-of-objects), and `deep_nesting`
  (8–64 levels; note serde_json hard-caps parse recursion at 128).
  `state_parse_and_lookup` adds a signal resolution so the gap vs
  `state_parse` is the lookup share (near zero — parse dominates).
  `state_large_object` sweeps object width 100–10k to expose the
  BTreeMap `O(log n)` key lookup.
* `handler_bench.rs → handler_large_state` — end-to-end `handle()` at
  1k/5k items as `render_only` (pre-parsed `Value`) vs
  `parse_and_render` (includes `from_str`). Shared byte-throughput
  denominator, so the curve gap is exactly the parse overhead.
  `handler_wide_state_signal` resolves one `{{signal}}` against a
  100–10k-key state to isolate wide-object lookup cost.

Run just these with:

```bash
cargo bench -p microsoft-webui-state   --bench state_bench   -- state_parse
cargo bench -p microsoft-webui-handler --bench handler_bench -- handler_large_state
```


### `streaming-resource` (counting allocator + getrusage)

`crates/webui/examples/streaming_resource_bench.rs` installs a custom
`GlobalAlloc` that exact-counts every `alloc`/`realloc` call. Why an
example, not a criterion bench? Criterion's harness allocates during
its sampling loop, which would pollute a counting allocator. Examples
run a clean process where every `alloc` we observe came from the code
under test (or its dependencies).

Reports per (path × scale):
- **allocs/run** — exact count from the custom allocator
- **bytes/run** — exact bytes requested from the allocator
- **wall µs/run** — `Instant::elapsed()` per iteration
- **user µs/run** — `getrusage(RUSAGE_SELF).ru_utime` delta
- **process RSS** — `ru_maxrss` high-water mark

This is the **only** bench in the suite that gives you exact
allocation numbers. Use it to verify "zero per-write allocation"
claims and to detect allocation-pressure regressions.

### `state-cpu` (parse vs render CPU + CPU%)

`crates/webui/examples/state_cpu_bench.rs` targets the exact cost the
load-test traces blamed: **JSON processing of the state object**. It
reuses the same cross-platform CPU reader as `streaming-resource`
(`getrusage` on Unix, `GetProcessTimes` on Windows) but reports the
CPU *split* across three stages at 1k / 5k / 10k catalog items:

- **parse** — `serde_json::from_str::<Value>` only (the addon's step 1)
- **render** — `WebUIHandler::render` on a pre-parsed `Value`
- **parse+render** — parse **then** render (the real per-request cost)

`parse+render − render ≈ parse CPU`, so the gap between those rows is
the CPU attributable to JSON parsing. Each row reports allocs/op,
bytes/op, wall µs/op, **user µs/op**, **sys µs/op**, **CPU%**
(`(user+sys)/wall` — ~100% means pure compute), and state throughput
(MiB/s normalized to the input JSON size). Use it to confirm where the
per-request CPU goes and to measure parser-side optimizations
(caching, splicing, a faster parser or map) before/after.

### `ffi-cpu` (Node addon, `process.cpuUsage()`)

`crates/webui-node/bench/ffi_cpu_bench.mjs` measures the **full FFI
per-request CPU** as the Node host actually pays it — the one number a
pure-Rust bench cannot produce, because it includes the boundary
costs: napi copying `stateJson` out of V8, and every rendered chunk
crossing back into JS (`content.to_owned()` per chunk + the `onChunk`
call). `cargo xtask bench ffi-cpu` builds the addon
(`cargo build -p microsoft-webui-node --release` →
`target/release/webui_node.<dll|so|dylib>`), loads it via
`process.dlopen`, compiles the contact-book protocol through the
addon's own `build()`, then loops `render()` over 1k / 5k / 10k JSON
state while sampling `process.cpuUsage()` (user + system µs).

Reports per scale: **user µs/op**, **sys µs/op**, wall µs/op, **CPU%**,
state MiB/s, output KiB/op, and chunks/op. Compare its `user µs/op`
against `state-cpu`'s `parse+render` row to see how much extra CPU the
FFI boundary itself adds on top of the Rust work. Requires Node.js on
`PATH`.

### `streaming-e2e-ttfb` (HTTP-level)

`crates/webui/examples/streaming_e2e_ttfb_bench.rs` spawns a real
actix-web server with `/buf` and `/stream` endpoints, then drives
both with the `awc` HTTP client. Reports min/p50/p99 for both TTFB
(time to first byte) and TTLB (time to last byte) at four
render-cost scenarios.

Faster than the browser bench (~10 s vs ~30 s) and doesn't need
Chromium installed. Use it as the smoke check before paying for the
full browser bench.

### `streaming-browser` (Playwright + Chromium)

`examples/integration/streaming-browser-bench/` is a separate package
with its own actix server and a Playwright spec that drives Chromium
through `PerformanceObserver`. Reports the **only** browser-perceived
metrics in the suite:

- **TTFB** — `responseStart - requestStart` from `PerformanceNavigationTiming`
- **FCP** — first-contentful-paint from `PerformanceObserver`
- **LCP** — largest-contentful-paint from `PerformanceObserver`
- **DCL** — `domContentLoadedEventEnd - startTime`
- **load** — `loadEventEnd - startTime`

This is the bench that answers "does streaming actually help users
see the page faster?" The HTTP-level benches prove the bytes get to
the wire faster; only this one proves Chrome paints faster.

The spec also asserts a **hard regression check**: at the 100 ms
render scenario, streaming TTFB must be ≥5× lower than buffered
TTFB. If that ever fails, something is fundamentally wrong with the
implementation.

## Recommended PR workflow

For any change touching `crates/webui/src/streaming.rs` or its
callers:

```bash
# 1. Establish baseline on the unmodified code
cargo xtask bench full --save-baseline before

# 2. Make your change

# 3. Compare
cargo xtask bench full --baseline before

# 4. Paste the four Δ%-tables into the PR description
```

For changes touching the handler / parser / state / protocol /
expressions crates:

```bash
cargo xtask bench all --save-baseline before
# … change …
cargo xtask bench all --baseline before
```

The criterion `--baseline` flag emits the per-bench `change:` lines
inline (e.g. `Performance has improved` / `regressed` / `within
noise threshold`).

## Where the data lives

* **Stdout** — every bench prints a human-readable table.
* **JSON snapshots** — non-criterion benches write to
  `target/bench-baselines/`.
* **Criterion HTML** — `target/criterion/report/index.html` for full
  PDF/CDF plots and per-baseline violin plots.

## Why so many benches?

Each layer measures a different thing. A change can:

- improve allocation count but regress wall-clock (allocator changes)
- improve micro-bench wall-clock but regress browser FCP (chunk-size
  changes that hurt parser progressive rendering)
- improve TTFB but introduce a memory leak (no cleanup of pool
  buffers on error paths)

Running the full suite catches all of these. Running just one layer
catches one third of them.

## Reproducibility tips

* **Close other applications** — CPU-intensive background work adds
  noise.
* **Plug in to power** (laptops) — battery savers throttle the CPU.
* **Pin to release builds** — `cargo bench` and `cargo xtask bench`
  always use release; debug builds are not representative.
* **Run on the same machine** — cross-machine baselines are not
  meaningful.
* **Compare medians (P50)**, not means — robust against thermal
  spikes.
* **Re-run if Dev% > 15%** in any criterion row.

## Authoring guidance

If you add a new performance-sensitive feature, also add a
benchmark. The bar:

1. **Criterion** if the unit-of-work is a single function call. Add a
   `[[bench]]` entry to the relevant crate's `Cargo.toml`.
2. **Example with `--save NAME`/`--compare NAME`** if you need
   process-wide measurement (custom allocator, getrusage, an HTTP
   server, etc.). Mirror the structure of
   `streaming_resource_bench.rs`.
3. **Playwright** if the metric is browser-perceived (paint, layout,
   hydration time). Mirror the structure of
   `examples/integration/streaming-browser-bench/`.

Wire it into `cargo xtask bench` so the standard before/after
workflow works without users needing to know per-bench invocation
details.
