# `streaming-browser-bench`

Browser-perceived metrics for the WebUI streaming SSR pipeline. This package
holds **two** independent benches:

1. **Transport bench** (`browser_metrics.spec.ts`) - buffered vs streamed
   delivery of byte-identical HTML.
2. **Progressive hydration matrix** (`hydration_matrix.spec.ts`) - the real
   streaming coordinator plus real `WebUIElement` hydration, measured
   across 1/3/10/100 boundaries against an ordinary one-shot control.

## Transport bench

This bench spins up a real actix-web server with two endpoints:

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

## Progressive hydration matrix

This bench needs no server: it bundles the *actual* framework sources in-memory
with esbuild (`__WEBUI_DEV__=false`, minified, IIFE/browser) into two fixtures
and drives the DOM directly via `page.setContent` + `page.addScriptTag`:

* the **ordinary** fixture imports `WebUIElement` from the framework default
  entry only;
* the **streaming** fixture imports `streaming-entry.ts` (the public
  coordinator entry) before the default entry.

Each fixture exposes an idempotent `window.__defineBenchIsland()` that defines
one instrumented `bench-island` class extending `WebUIElement` (a real two-text-
binding template). The subclass accumulates real component mount/hydration CPU on
a window global so the CPU comparison is independent of boundary parsing /
coordinator overhead:

* ordinary (unmarked) roots time `super.connectedCallback()`;
* streamed (`data-ws`) roots do the cheap deferral uncounted and time
  `super.$activateDeferredSSR(state)` instead.

Each hook also samples `usedJSHeapSize` (after stopping its CPU timer) into a
window global, so peak heap is captured *while components commit*, and increments
a per-instance successful-hydration count gated on the base class's real
`$hydrated` flag - so an early return (deferral, missing-metadata warn, or no-op
activation) can never inflate it. After all timed/peak measurements stop, the
driver runs an independent **reactive probe**: it calls each root's real public
`setState` with a shared sentinel `label`, which flushes synchronously, and
verifies the bound `<span>` re-rendered. Both the hydration count and the
reactive-verified count must equal 1500 (`TOTAL_ROOTS`) for every arm, proving
every root genuinely hydrated rather than merely surviving scaffold removal. (The
probe caught two silent breakages after a framework coordinator rewrite - see
Known limitations.)

Every scenario delivers the **same** fixed total of real `bench-island` SSR
roots (1500) and the **same** total projected state value bytes (24 KiB of
`label` values); only the boundary count (and marker layout) changes. "Projected
state value bytes" counts the streamed `label` values a real app would ship, and
deliberately excludes unavoidable per-boundary protocol/property overhead (the
`[1,seq,kind,target,{...}]` envelope framing, the first-boundary `templates` block,
and the tiny fixed `note` property) - that overhead is inherent to having more
boundaries, not equal work to hold constant. It is projected-state value bytes,
not total wire bytes.

Each streamed boundary is real wire format - `<!--wb:N-->` markers + SSR roots
carrying `data-ws` + an inert `[data-webui-boundary]` JSON script + a
`<webui-hydrate>` sentinel - appended one at a time, with the driver spinning the
coordinator's microtask pump until that boundary's scaffolding is removed (a
deterministic "committed + cleaned" signal) before the next, then a terminal
envelope. Consecutive chunks enter through separate `MessageChannel` tasks,
matching browser response-chunk scheduling without the nested timer clamp that
would add synthetic delay at 100 boundaries. The control uses the real inert
`#webui-data` bootstrap (present in the base document so the framework's lazy
loader latches on the real block); only its SSR roots are inserted at run time
(after the baseline heap sample) so its empty->populated peak-heap transition
matches the streaming arms.

Coverage includes flat and deeply nested marker ranges (one root at each
successive `<div>` depth) and the boundary-before-definition race (the class
stays undefined for the first boundary, exercising the O(unique tag) waiter
path).

### Metrics collected per run

| Metric | Source |
|---|---|
| component hydration CPU sum | instrumented `bench-island` subclass |
| successful-hydration count + reactive-verified count | instrumented subclass (`$hydrated`-gated) + post-measurement `setState` probe; both must equal 1500 |
| exact boundary-state delivery | correctness-only wrapper around the real deferred activation hook with distinct state per boundary |
| total scenario elapsed | `performance.now()` (streaming: append+commit pipeline; ordinary: the synchronous hydration burst, excluding one-time body parse) |
| median + p95 | across `RUNS` repeats |
| peak JS heap delta | `performance.memory.usedJSHeapSize` sampled inside the hydration hooks + per-boundary (`--enable-precise-memory-info`) |
| forced-GC retained heap delta | CDP `HeapProfiler.collectGarbage` + `Runtime.getHeapUsage` before/after |
| max long task | `PerformanceObserver('longtask')` (when supported) |
| bundle bytes | exact minified + gzip for both fixtures |

`RUNS` defaults to 5 for normal runs (cheap; note that with only 5 samples
nearest-rank p95 == max). Strict/enforced runs use at least 20 samples so p95 is
a real tail statistic; override with `WEBUI_STREAMING_HYDRATION_RUNS=<n>` (floored
at 20 under enforce).

Every production arm is warmed once and discarded before measurement. Each
round then runs every arm exactly once, rotating and reversing arm order across
rounds so no scenario owns a fixed hot or cold position. CPU and elapsed gates
compare aligned samples from the same round as paired deltas; they do not compare
an interleaved current run against a stale standalone baseline.

The primary scaling arms all receive the same empty metadata checkpoint before
their 1/3/10/100 content checkpoints. This keeps custom-element construction
identical across arms: every measured root is parser-created after template
metadata defines the class. Without that setup, B=1 upgrades every root after
parsing while larger arms mix upgraded and parser-created roots, so the CPU gate
would measure a changing construction mode rather than boundary scaling. The
separate eager/race coverage cases keep metadata co-located with real roots and
exercise the production first-checkpoint behavior.

### Deterministic vs strict gates

Always enforced (hard guarantees, noise-free): equal live root counts, **every
root proven hydrated** (successful-hydration count and reactive `setState`-probe
count both equal 1500), zero residual scaffolding (scripts, sentinels, `wb:`
comments, `[data-ws]`), no globally-published streamed state
(`window.__webui.state` stays unset), the ordinary bundle contains no coordinator
tokens (`webui-hydrate` / `data-webui-boundary`), measured component CPU is
non-zero, distinct boundary-local states reach only their own real activation
hooks, and the streaming entry adds no more than 12 KiB minified / 4.125 KiB
gzip. Esbuild output is deterministic and the cap retains under 4% headroom, so
further growth still fails.

Opt-in via `WEBUI_STREAMING_HYDRATION_ENFORCE=1` (noisy, off by default), each
printing its effective cap:

* component-hydration median for every streamed arm <= the single-boundary
  streamed one-shot median * 1.05 + 0.25 ms floor. This compares the same direct
  boundary-state activation path, so a slower ordinary bootstrap cannot mask
  boundary-scaling regressions;
* retained-heap N=1/10/100 slope <= max(64 KiB, 2%) after forced GC (this is the
  tightest gate: observed slope ~45-57 KiB sits just under the 64 KiB floor, so
  it is the most likely to need a higher floor on noisier CI hardware);
* peak heap must not grow with boundary count - every arm within max(512 KiB,
  15%) of the single-boundary (largest-boundary) working set. Referenced against
  streaming itself, not the one-shot control, because the coordinator legitimately
  carries transient scaffolding the control never allocates;
* every primary run must produce both a peak-heap sample and a forced-GC retained
  sample; missing browser/CDP memory instrumentation fails an enforced run rather
  than silently skipping its memory assertions;
* coordinator marginal elapsed growth from 1 to 100 boundaries <=
  max(0.25 ms, 1% of the single-boundary elapsed) per added boundary.

Under baseline compare (`WEBUI_BENCH_COMPARE`), retained- and peak-heap deltas
are reported per scenario alongside CPU/elapsed. Retained regresses when it grows
by more than max(64 KiB, 10%) of the baseline; peak when it grows by more than
max(512 KiB, 15%). The absolute floors are always applied, so a *uniform* memory
regression that leaves the within-run N=1/10/100 slope flat is still caught, and
growth from a zero/small baseline is never masked by a percentage of zero. A
sample that is null on either side is reported `n/a` and skipped. Under enforce, a
compare regression (CPU, elapsed, retained, or peak) fails the run.

These are structured and documented separately so timing/heap noise never fails a
normal run.

### Known limitations

* `performance.memory` is Chromium-only and coarse/bucketed, so peak-heap deltas
  are noisy (the in-hook sampling makes them *truthful during commit* but not
  precise); the forced-GC CDP retained delta is the authoritative memory metric.
* On this dev machine `longtask` reads `null` for the streaming arms (the
  measured pipeline remains below 50 ms) while the one-shot control's synchronous
  1500-root hydration can trip a single long task - the concrete streaming
  main-thread-jank win - but the metric is only meaningful where the observer is
  supported.
* The ordinary and streamed component timers cover their real production entry
  points (`connectedCallback` vs direct deferred activation), so their absolute
  CPU values are useful workload signals but are not interchangeable baselines.
  Boundary-scaling gates therefore use the single-boundary streamed arm; saved
  before/after snapshots catch regressions shared by every streamed arm.
* This harness injects the framework bundle after the page loads and streams
  boundaries from script. To interoperate with the coordinator it (a) keeps the
  inert `#webui-data` bootstrap in the base document (the framework's lazy loader
  latches its "already loaded" guard on a macrotask, so an empty DOM at that point
  would poison it and leave roots un-hydrated), and (b) pins `document.readyState`
  to `'loading'` on the streaming page so the coordinator's truncation guard waits
  for a `DOMContentLoaded` that never fires - the terminal envelope ends the
  stream instead. Both are inert to the hydration path itself. The reactive
  `setState` probe was added precisely because, without it, either issue would
  silently leave roots un-hydrated while live-root-count and aggregate CPU still
  passed.


## Run

```bash
# Transport bench (Chromium driver, ~30 s)
cargo xtask bench streaming-browser

# Or directly:
cd examples/integration/streaming-browser-bench
pnpm test              # transport bench
pnpm test:hydration    # progressive hydration matrix
pnpm typecheck         # tsc --noEmit for this package
```

## Before/after comparison

```bash
# 1. Snapshot current numbers as 'before'
cargo xtask bench streaming-browser --save-baseline before

# 2. Make change …

# 3. Compare
cargo xtask bench streaming-browser --baseline before
```

The transport bench writes `target/bench-baselines/browser-<name>.json` and the
compare phase prints a Δ%-table for TTFB, FCP, LCP, and load.

(Underneath, this maps to env vars `WEBUI_BENCH_SAVE` and
`WEBUI_BENCH_COMPARE` consumed by the spec; you can also set them
directly when running `pnpm test`.)

The hydration matrix reuses the same env vars but writes a **distinct** file so
the two benches never clobber each other:

```bash
WEBUI_BENCH_SAVE=before    pnpm test:hydration
WEBUI_BENCH_COMPARE=before pnpm test:hydration
# Compare only fails the run when the strict flag is also set:
WEBUI_STREAMING_HYDRATION_ENFORCE=1 WEBUI_BENCH_COMPARE=before pnpm test:hydration
```

Hydration snapshots live at
`target/bench-baselines/browser-hydration-<name>.json` and record median/p95
CPU and elapsed, peak/retained heap, long task, and the bundle byte sizes. The
compare phase prints a per-scenario delta table with CPU %, elapsed %, and signed
retained- and peak-heap KiB deltas. A strict comparison requires a compatible
baseline and checks it before running the measurement matrix; a missing or stale
baseline fails with the command needed to create a new one.


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
