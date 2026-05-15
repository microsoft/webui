# `streaming-browser-bench`

Browser-perceived metrics for the WebUI streaming SSR pipeline.

This package spins up a real actix-web server with two endpoints:

* `/buf?delay_us=N` — buffered render (whole HTML in one HTTP chunk)
* `/stream?delay_us=N` — streaming render (`StreamingWriter` +
  lock-free `ChunkPool`)

Both endpoints serve **byte-identical HTML**; only the delivery
mechanism differs. Playwright drives Chromium against both endpoints
and captures real browser metrics via `PerformanceNavigationTiming` and
`PerformanceObserver`.

The `delay_us` query parameter injects a per-`write()` artificial
sleep on the server, simulating slower-rendering pages so we can
measure the streaming win at realistic render times (~5 ms /
~25 ms / ~100 ms / ~250 ms).

For the bench-suite-wide picture, see
[`BENCHMARKS.md`](../../../BENCHMARKS.md) at the repo root.

## Run

```bash
# Full bench (Chromium driver, ~30 s)
cargo xtask bench streaming-browser

# Or directly:
cd examples/integration/streaming-browser-bench
pnpm test
```

## Before/after comparison

```bash
# 1. Snapshot current numbers as 'before'
cargo xtask bench streaming-browser --save-baseline before

# 2. Make change …

# 3. Compare
cargo xtask bench streaming-browser --baseline before
```

Snapshots are written to
`target/bench-baselines/browser-<name>.json`. The compare phase
prints a Δ%-table for TTFB, FCP, LCP, and load.

(Underneath, this maps to env vars `WEBUI_BENCH_SAVE` and
`WEBUI_BENCH_COMPARE` consumed by the spec; you can also set them
directly when running `pnpm test`.)

## What it measures

| Metric | Source | What it tells you |
|---|---|---|
| **TTFB** | `responseStart - requestStart` | when the first byte hit the browser |
| **FCP** | `paint` `PerformanceObserver` | when the user first sees something |
| **LCP** | `largest-contentful-paint` `PerformanceObserver` | when the main content appeared |
| **DCL** | `domContentLoadedEventEnd - startTime` | when DOM was parsed |
| **load** | `loadEventEnd - startTime` | when the page fully loaded |

## Hard regression guard

The spec asserts: at the 100 ms render scenario, streaming TTFB
must be ≥5× lower than buffered TTFB. If that ever fails, something
is fundamentally wrong with the streaming pipeline.

## Why a separate package?

The browser bench has different requirements from the criterion +
example benches in `crates/webui/`:

- needs Playwright + Chromium installed
- spawns a long-lived HTTP server
- measurements come from JavaScript, not Rust

Keeping it as a workspace member lets `cargo build` validate the
server compiles, while the actual run lives behind `pnpm test` (or
`cargo xtask bench streaming-browser`).

## Treat as signal vs noise

Browser metrics are inherently noisier than micro-benches:

| Metric | Noise threshold |
|---|---|
| TTFB | ±5 ms (loopback adds variability) |
| FCP / LCP | ±5 ms |
| DCL / load | ±10 ms |

Treat differences ≥15% as real signal; smaller deltas should be
re-measured with more iterations.
