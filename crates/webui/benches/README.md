# `microsoft-webui` benches

Two criterion benches in this directory:

* **`contact_book_bench.rs`** — end-to-end render of the
  contact-book-manager template at 10 / 100 / 1 000 contacts. Measures
  protocol parsing and full-render wall-clock without/with the FAST 2.x
  hydration plugin.
* **`streaming_bench.rs`** — writer-path wall-clock comparison: `String`
  baseline vs `StreamingWriter` vs `StreamingWriter + RenderOptions
  inject` (per-render head/body inject via the handler's signal-based
  hook) vs `String + post-injection` (the legacy livereload path the
  streaming module replaces). Includes a separate `ttfb` group that
  measures time-to-first-chunk for the streaming path.

Two **examples** (in `crates/webui/examples/`) round out the suite:

* **`streaming_resource_bench.rs`** — exact allocation count, bytes
  allocated, getrusage user/system CPU time, and peak RSS via a custom
  `GlobalAlloc`. The only bench in the workspace that gives exact
  allocation numbers. On non-Unix targets, getrusage-backed CPU/RSS
  counters report zero while allocation and wall-clock measurements still run.
* **`streaming_e2e_ttfb_bench.rs`** — HTTP-level TTFB through a real
  actix-web server.

A separate Playwright package handles browser-perceived metrics:

* **`examples/integration/streaming-browser-bench/`** — TTFB / FCP /
  LCP / DCL / load measured by Chromium via `PerformanceObserver`.

For the cross-bench picture and recommended workflow, see
[`BENCHMARKS.md`](../../../BENCHMARKS.md) at the repo root.

## Quick reference

| Command | What it does |
|---|---|
| `cargo xtask bench contact-book` | run the criterion contact-book bench |
| `cargo xtask bench streaming` | run the criterion streaming bench |
| `cargo xtask bench streaming-resource` | run the resource-counting example |
| `cargo xtask bench streaming-e2e-ttfb` | run the HTTP-level TTFB example |
| `cargo xtask bench streaming-browser` | run the Playwright browser-metrics test |
| `cargo xtask bench full` | run all four streaming-related benches in sequence |
| `cargo xtask bench all` | run every criterion bench in the workspace |

All commands accept `--save-baseline NAME` to record current numbers
and `--baseline NAME` to compare against a saved baseline:

```bash
cargo xtask bench full --save-baseline before
# … make change …
cargo xtask bench full --baseline before
```

Snapshots live under `target/bench-baselines/`. Criterion baselines
live under `target/criterion/<bench>/<name>` (criterion's native
location).

## Reading the results

Each bench prints a human-readable table to stdout. When `--baseline
NAME` is set, a Δ%-table is printed comparing current to baseline:

```
Diff vs baseline 'before' (saved 30s ago)
| row                                 |  allocs Δ% |   bytes Δ% | user_cpu Δ% |
|-------------------------------------|------------|------------|-------------|
| string/100                          |       0.0% |       0.0% |        1.2% |
| streaming/100                       |       0.0% |       0.0% |       -2.1% |
| streaming+inject(opts) POOLED/100   |       0.0% |       0.0% |       -3.4% |
```

Negative Δ% = improvement; positive = regression.

## Detecting regressions

| Source | Treat as noise | Treat as signal |
|---|---|---|
| criterion wall-clock | < ±2% | > ±5% |
| streaming-resource alloc count | exact — any change matters | any non-zero |
| streaming-resource bytes/CPU | < ±2% | > ±5% |
| streaming-e2e-ttfb (loopback) | < ±10% | > ±20% |
| streaming-browser (real Chromium) | < ±5% | > ±15% |

For criterion's HTML reports with PDF/CDF plots and violin
comparisons, open `target/criterion/report/index.html`.

## Tips for reliable measurements

- **Close other applications** — background CPU adds noise.
- **Plug in laptops** — battery savers throttle.
- **Always release mode** — `cargo bench` and `cargo xtask bench`
  guarantee this; never rely on debug numbers.
- **Compare P50 over Avg** — median is more robust to outliers.
- **Re-run if Dev% > 15%** for any criterion row.
- **Reset baseline:** `rm -rf target/criterion target/bench-baselines`
  and re-run.
