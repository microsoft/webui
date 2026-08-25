# `node-addon-bench`

Runtime metrics for the WebUI Node.js native addon.

The runner calls the public `@microsoft/webui` package from a real Node.js
process, so every measured render crosses the same V8/N-API boundary as a
production Node host. It uses the Contact Book application and the same
10/100/1000-contact scales as the Rust end-to-end benchmark.

For the bench-suite-wide picture, see
[`BENCHMARKS.md`](../../../BENCHMARKS.md) at the repository root.

## Run

```bash
# Self-contained release benchmark (recommended)
cargo xtask bench node-addon

# Short correctness/wiring check against an already-built addon
pnpm --filter node-addon-bench bench:quick
```

`cargo xtask bench node-addon` builds the release addon and
`@microsoft/webui` package before starting Node. When running the package
script directly, build them first:

```bash
cargo build --release -p microsoft-webui-node
pnpm --filter @microsoft/webui build
pnpm --filter node-addon-bench bench
```

A debug addon is rejected by normal benchmark commands. The runner's
`--allow-debug` flag is only for local smoke checks; never use those numbers in
a performance comparison.

## Before/after comparison

```bash
# 1. Snapshot the current release numbers
cargo xtask bench node-addon --save-baseline before

# 2. Make the change ...

# 3. Compare P50 and output shape with the snapshot
cargo xtask bench node-addon --baseline before
```

Snapshots are written to
`target/bench-baselines/node-addon-<name>.json`. The compare phase prints P50
and output-size deltas plus streaming chunk counts. It also warns when the
platform, Node major version, route, or protocol size differs from the
baseline.

Underneath, xtask maps the baseline flags to `WEBUI_BENCH_SAVE` and
`WEBUI_BENCH_COMPARE`. They can also be set when invoking `pnpm run bench`
directly.

## What it measures

| Case | Boundary included | What it tells you |
|---|---|---|
| `protocol/new` | Node `Buffer` -> N-API -> protobuf decode/index | `Protocol` construction after the addon is loaded |
| `prepare-state/N` | Pre-serialized JSON -> N-API -> JSON parse -> immutable native snapshot | One-time preparation cost for repeated state |
| `render/json-string/N` | JS string -> N-API -> JSON parse -> Rust render -> external Node `Buffer` | canonical buffered response when state is already serialized |
| `render/object/N` | `JSON.stringify` plus the JSON-string path | public-package cost for the common object-state API |
| `render/prepared/N` | Prepared native state -> Rust render -> external Node `Buffer` | repeated render cost without serialization or JSON parsing |
| `render-stream/...`, `first-callback` | JS string -> N-API -> JSON parse -> render to first 16 KiB -> JS callback | latency until JavaScript receives the first chunk |
| `render-stream/...`, `total` | the same input path plus all chunks and callbacks | complete streaming callback-path cost |

Streaming uses pre-serialized state so callback-path changes are not conflated
with `JSON.stringify`. The first-callback metric is an **in-process callback
latency**, not HTTP TTFB or socket-flush latency. Use `streaming-e2e-ttfb` or
`streaming-browser` for wire-level and browser-perceived metrics.

## Not measured

- addon/package module loading or reading protocol bytes from disk
- the `build()` API (template compilation happens only during setup)
- HTTP writes, socket flushes, or user callback work such as `response.write()`
- Node event-loop delay, concurrent request throughput, RSS, or V8 heap usage

Those need process/server-level benchmarks rather than this synchronous API
runner.

## Methodology and correctness guards

- Builds the live Contact Book templates once before sampling.
- Loads the application's checked-in `data/state.json` and repeats its contacts
  to produce the 10/100/1000-contact workloads.
- Renders the `/contacts` route so both state and HTML output scale with the
  workload.
- Reuses one decoded `Protocol` for all hot render cases.
- Warms each operation before collecting samples.
- Runs garbage collection once before each sample group when Node exposes it;
  GC is not forced between individual operations.
- Keeps one prepared snapshot per workload live throughout render sampling.
  Preparation rows allocate fresh snapshots, so their timing includes natural
  V8 GC pressure after the group-level collection.
- Collects at least 20 samples per row and targets 750 ms of measurement time.
- Reports workload sizes/chunk counts plus min, P50, P95, P99, and
  mean-derived ops/s.
- Verifies object, JSON-string, and prepared inputs produce byte-identical
  buffers.
- Verifies concatenated streaming chunks exactly equal buffered output.
- Rejects empty protocols, missing chunks, malformed baselines, unsafe baseline
  names, and accidental debug measurements.

Each prepared handle retains a parsed Rust state tree until JavaScript garbage
collection. The addon reports an approximate external-memory size to V8 so the
collector can account for that pressure, but this runner does not report native
allocation counts or RSS. Interpret `render/prepared/N` as the CPU benefit for a
deliberately retained immutable snapshot, not as a free per-request optimization.

For raw machine-readable output:

```bash
pnpm --filter node-addon-bench bench:json
```

## Why a separate integration package?

This benchmark cannot be a Criterion target: calling the Rust functions from a
Rust harness would bypass V8, Buffer ownership transfer, N-API, and callback
crossings. Keeping it next to `streaming-browser-bench` makes the external
runtime requirement explicit while letting pnpm manage the public package
under test.

## Treat as signal vs noise

Node process metrics are noisier than isolated Criterion measurements because
V8 and garbage collection share the process. On the same machine and Node
major version, treat P50 changes below +/-5% as noise and changes above +/-10%
as a signal. Re-run changes in between; use P95/P99 to investigate tail
behavior rather than as a hard regression gate.
