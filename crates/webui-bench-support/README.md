# microsoft-webui-bench-support

Shared measurement and reporting harness for WebUI's **resource benchmarks** —
the CPU / memory / throughput / latency harnesses that live as
`examples/*_bench.rs` binaries and drive `cargo xtask bench`.

This crate is **dev-only**. Add it under `[dev-dependencies]`:

```toml
[dev-dependencies]
microsoft-webui-bench-support = { path = "../webui-bench-support" }
```

## Why it exists

WebUI uses [Criterion](https://docs.rs/criterion) for statistical wall-clock
micro-benchmarks (`benches/*.rs`). Criterion deliberately measures **only
wall-clock time**, but the production problem — the Node SSR host burning ~60 %
CPU vs ~25 % for a JS control at 150 RPS — is a **CPU-time** and **allocation**
problem. The resource benches therefore roll their own measurement, and this
crate is the single, unit-tested home for the primitives they share so every
bench reports the same four dimensions the same way.

| dimension      | primitive                                                        |
|----------------|------------------------------------------------------------------|
| **cpu**        | `cpu::ProcessUsage` — user + system µs, peak RSS                  |
| **memory**     | `alloc::CountingAllocator` — exact allocations + bytes            |
| **latency**    | `measure::Measurement` wall time, `measure::percentile`          |
| **throughput** | `measure::PerIter` — ops/s and MiB/s                             |

Regression gating (`--save` / `--compare` JSON baselines) is provided
generically by `baseline`; consistent tables by `report`.

## Layering

```text
Criterion            → statistical wall-clock micro-benchmarks (benches/*.rs)
webui-bench-support  → CPU/mem/throughput/latency resource benches (examples/*_bench.rs)
```

## Shape of a resource bench

```rust,ignore
use webui_bench_support::alloc::CountingAllocator;
use webui_bench_support::{bench, baseline, report::Table};
use std::time::Duration;

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator::new();

let m = bench(Duration::from_millis(200), input_bytes, 50, 200_000, || {
    render_once();
});
let pi = m.per_iter();
// pi.user_us, pi.cpu_pct, pi.ops_per_s, pi.work_mib_s, pi.bytes, pi.allocs …
```

See `crates/webui/examples/state_cpu_bench.rs` for a full consumer, and
`BENCHMARKS.md` at the repo root for the benchmark catalog.
