# Performance

WebUI is designed for performance at every layer of the stack — from build-time
compilation to binary serialization to streaming output. This page explains the
design decisions, shares real benchmark data, and shows you how to measure
performance in your own applications.

## Performance by Design

Five architectural choices keep WebUI fast without any tuning:

- **No recursion** — all algorithms are iterative, making them stack-safe even
  for large documents with deeply nested components.
- **No regular expressions** — tree-sitter handles parsing and iterative
  matchers handle route resolution, avoiding backtracking overhead entirely.
- **Minimal runtime computation** — templates are compiled to a binary protocol
  at build time. The server never parses template syntax on a live request.
- **Buffer consolidation** — adjacent static content is merged into single
  fragments during compilation, reducing the number of write calls at render
  time.
- **Protocol Buffers** — templates and the render protocol are serialized to a
  compact binary format (`protocol.bin`) that decodes static state significantly faster than
  JSON-based template representations, keeping only dynamic-time state in JSON.

## SSR Performance Showdown

**Methodology:** [autocannon](https://github.com/mcollina/autocannon), 100
concurrent connections, 10-second duration, 2-second warmup. The workload
renders ~2,400 tiles per request — a realistic stress test that exercises
loops, conditionals, and component composition.

| Framework           | Avg Latency | p50    | p99    | Req/Sec   | Throughput   |
| ------------------- | ----------- | ------ | ------ | --------- | ------------ |
| **WebUI (Rust)**    | **21.7 ms** | **18 ms** | **52 ms** | **4,511** | **684 MB/s** |
| Fastify (Node.js)   | 93.4 ms     | 92 ms  | 118 ms | 1,061     | 209 MB/s     |
| React SSR (Node.js) | 179.2 ms    | 180 ms | 210 ms | 552       | 78.5 MB/s    |

WebUI is **4.3× faster** than Fastify and **8.2× faster** than React SSR.
Notably, WebUI's p99 latency (52 ms) is lower than Fastify's *median* (92 ms),
meaning WebUI's worst case outperforms Fastify's typical case.

## Contact Book Benchmark

A real-world application benchmark that exercises components, `for`-loops,
`if`-conditions, and nested state. The contact book renders a list of contacts
with avatars, metadata, and action buttons.

| Workload        | Render Time | Output Size |
| --------------- | ----------- | ----------- |
| Protocol parse  | 0.05 ms     | 28 KB binary |
| 10 contacts     | 0.65 ms     | 25 KB HTML  |
| 100 contacts    | 4.94 ms     | 56 KB HTML  |
| 1,000 contacts  | 57.5 ms     | 363 KB HTML |

Hydration plugin overhead is minimal: ~2–3% (59.5 ms vs 57.5 ms at 1,000
contacts). The cost of embedding hydration markers is negligible compared to
the rendering work itself.

## Why WebUI is Fast

Each layer of the architecture contributes to the overall performance profile:

- **Build-time compilation.** Template parsing, component discovery, and
  expression compilation all happen once during `webui build` (or on the fly
  with `webui serve` in development). At runtime, the server only performs
  state interpolation against a pre-compiled binary protocol — no syntax
  parsing, no AST walking.

- **Protocol Buffers.** The handler deserializes a compact binary payload
  instead of parsing template syntax on every request. Protocol Buffer
  deserialization is an order of magnitude faster than JSON parsing for
  equivalent payloads.

- **Streaming output.** The `ResponseWriter` trait enables flushing HTML chunks
  to the client as they are produced. This reduces time-to-first-byte and
  avoids buffering the entire response in memory.

- **No JavaScript runtime.** There is no V8, no garbage collector pauses, and
  no JIT warmup. The hot path is pure compiled Rust with predictable, low-
  latency execution.

- **Targeted updates.** On the client side, path-indexed binding updates touch
  only the affected DOM nodes — not entire subtrees. This keeps hydration and
  reactive updates fast even in large documents.

## Performance Rules

The following rules are enforced throughout the WebUI codebase to maintain
consistent performance:

- **No cloning large state trees** — pass by reference and capture borrows.
  Cloning a state tree duplicates memory and adds allocation pressure.
- **No `format!()` in writer output** — use sequential `writer.write()` calls.
  `format!()` allocates a temporary `String` on every invocation.
- **No `.collect::<Vec<_>>()` on splits** — iterate directly over the iterator.
  Collecting into a `Vec` allocates heap memory unnecessarily.
- **No `String::from(ch)` in escape loops** — use stack-allocated buffers.
  Converting a single character to a `String` is a heap allocation per
  character.
- **No per-request template re-parsing** — load the compiled protocol once at
  startup and reuse it for every request.

## Running Benchmarks

Use the built-in benchmark suite to measure performance on your own hardware:

```bash
# Full benchmark suite (recommended)
cargo xtask bench all

# Individual crate benchmarks via xtask
cargo xtask bench parser
cargo xtask bench handler
cargo xtask bench expressions
cargo xtask bench protocol
cargo xtask bench state

# Contact book end-to-end benchmark
cargo bench -p microsoft-webui --bench contact_book_bench

# Results with HTML reports
ls target/criterion/report/index.html
```

Each benchmark uses [Criterion.rs](https://github.com/bheisler/criterion.rs)
for statistical rigor — results include confidence intervals, outlier
detection, and comparison against previous runs.

## Measuring Hydration Performance

WebUI emits a `webui:hydration-complete` event after all components have been
hydrated on the client. Use the Performance API to inspect per-component
timing:

```typescript
window.addEventListener('webui:hydration-complete', () => {
  for (const entry of performance.getEntriesByType('measure')) {
    if (entry.name.startsWith('webui:hydrate:')) {
      console.log(`${entry.name}: ${entry.duration.toFixed(1)}ms`);
    }
  }
});
```

Each hydrated component produces a `webui:hydrate:<ComponentName>` measure
entry, making it straightforward to identify slow components and optimize them
individually.

## Learn More

- [SSR showdown source](https://github.com/microsoft/webui/tree/main/examples/integration/ssr-performance-showdown) — full benchmark harness and reproduction steps
- [Contact book benchmark](https://github.com/microsoft/webui/tree/main/crates/webui/benches) — real-world application benchmark
- [DESIGN.md](https://github.com/microsoft/webui/blob/main/DESIGN.md) — architectural performance principles
