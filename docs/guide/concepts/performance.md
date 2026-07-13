# Performance

WebUI is designed for performance at every layer of the stack - from build-time
compilation to binary serialization to streaming output. This page explains the
design decisions, shares real benchmark data, and shows you how to measure
performance in your own applications.

## Performance by Design

Five architectural choices keep WebUI fast without any tuning:

- **No recursion** - all algorithms are iterative, making them stack-safe even
  for large documents with deeply nested components.
- **No regular expressions** - deterministic scanners handle parsing and
  iterative matchers handle route resolution, avoiding backtracking overhead entirely.
- **Minimal runtime computation** - templates are compiled to a binary protocol
  at build time. The server never parses template syntax on a live request.
- **Buffer consolidation** - adjacent static content is merged into single
  fragments during compilation, reducing the number of write calls at render
  time.
- **Protocol Buffers** - templates and the render protocol are serialized to a
  compact binary format (`protocol.bin`) that decodes static state significantly faster than
  JSON-based template representations, keeping only dynamic-time state in JSON.

## SSR Performance Showdown

**Methodology:** [autocannon](https://github.com/mcollina/autocannon), 100
concurrent connections, 10-second duration, 2-second warmup. The workload
renders ~2,400 tiles per request - a realistic stress test that exercises
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

Hydration marker overhead is small in this fixture. Browser state is a separate
cost: serializing a large server state object can dominate a small render.
WebUI sends only the state needed by authored components on the active route.

## Hydration Startup and Protocol Reuse

The initial page contains only top-level `@observable` and `@attr` values needed
by reachable authored components. Template-only values and inactive routes do
not enlarge startup state.

Representative release-mode Criterion results:

| Workload | Before | After | Improvement |
|----------|-------:|------:|------------:|
| Serialize pre-parsed 1 MiB initial state not needed by client code | 1.5415 ms | 1.3298 us | 1,159x |
| 1 MiB partial-navigation state | 4.5930 ms | 573.85 us | 8.00x |

The initial-state benchmark starts from an already parsed Rust value. Hosts
that accept JSON strings still pay JSON parsing cost. Preparing
`protocol.bin` avoids repeated protocol decoding, not state parsing.

Example payloads show the same reduction:

| Initial route | Previous state JSON | Decorator-only state JSON | Reduction |
|---------------|--------------------:|--------------------------:|----------:|
| Routes home, behavior but no decorators | 191 bytes | 2 bytes (`{}`) | 98.95% |
| Contact book home | 103 bytes | 20 bytes (`{"totalFavorites":5}`) | 80.58% |

Native hosts should also prepare `protocol.bin` once at startup:

- Rust: construct `PreparedProtocol`
- C: use `webui_protocol_create` and the `*_prepared` functions
- .NET: construct `PreparedProtocol` and use the matching handler overloads
- WASM: construct the exported `PreparedProtocol`
- Node: reuse the same protocol `Buffer` and plugin; `@microsoft/webui` caches
  its plugin-bound native representation by buffer identity

Do not read `protocol.bin` into a new Node `Buffer` for every request. A stable
buffer lets the package reuse protobuf decoding and deterministic indices.

::: warning Browser state is client-facing
Never put credentials, private tokens, or other secrets in state rendered to
the browser.
:::

## Why WebUI is Fast

Each layer of the architecture contributes to the overall performance profile:

- **Build-time compilation.** Template parsing, component discovery, and
  expression compilation all happen once during `webui build` (or on the fly
  with `webui serve` in development). At runtime, the server only performs
  state interpolation against a pre-compiled binary protocol - no syntax
  parsing, no AST walking.

- **Protocol Buffers.** The handler consumes a compact binary payload instead
  of parsing template syntax. Prepared host APIs decode the protocol and build
  deterministic indices once at startup rather than repeating that work per
  request.

- **Streaming output with backpressure.** The `webui::streaming::StreamingWriter`
  coalesces handler writes into ~4 KB chunks and pushes them through a
  bounded `tokio::mpsc` channel, so the browser starts parsing while
  the server is still serializing. A shared lock-free `ChunkPool`
  recycles chunk buffers across requests (zero per-flush allocation
  in steady state), and a configurable flush deadline bounds the
  slow-loris DoS surface. Real-Chromium measurement on a 250 ms render
  shows TTFB drops from 265 ms (buffered) to 0.4 ms (streaming), with
  FCP / LCP from 284 ms to 56 ms. See `BENCHMARKS.md` and
  `examples/integration/streaming-browser-bench/`.

- **No JavaScript runtime.** There is no V8, no garbage collector pauses, and
  no JIT warmup. The hot path is pure compiled Rust with predictable, low-
  latency execution.

- **Targeted updates.** On the client side, path-indexed binding updates touch
  only the affected DOM nodes - not entire subtrees. This keeps hydration and
  reactive updates fast even in large documents.

## Light DOM vs Shadow DOM

Shadow DOM provides style encapsulation but has a performance cost. Benchmark
data from a 2,400-component email client:

| Metric | Shadow DOM | Light DOM | Improvement |
|--------|-----------|-----------|-------------|
| First Contentful Paint | baseline | **26% faster** | fewer shadow root constructions |
| Layout Operations | baseline | **60% fewer** | no shadow boundary recalculations |

### When to Use Each

**Shadow DOM** (default) - use when:
- Style encapsulation is important (shared component libraries)
- Components are used in contexts where CSS conflicts are likely
- You need slot-based composition

**Light DOM** - use when:
- Performance is critical (high-component-count pages)
- Components are leaf nodes (list items, cards, badges)
- You control the full page CSS and don't need encapsulation

### Switching to Light DOM

Build with the `--dom=light` flag:

```bash
webui build ./src --out ./dist --dom=light
```

In Rust handlers, use `DomStrategy::Light`:

```rust
let options = RenderOptions::new("index.html", "/")
    .with_dom_strategy(DomStrategy::Light);
```

CSS differences:
- Shadow DOM: `:host { display: block; }`
- Light DOM: `my-component { display: block; }` (use the tag name)

## Performance Rules

The following rules are enforced throughout the WebUI codebase to maintain
consistent performance:

- **No cloning large state trees** - pass by reference and capture borrows.
  Cloning a state tree duplicates memory and adds allocation pressure.
- **No `format!()` in writer output** - use sequential `writer.write()` calls.
  `format!()` allocates a temporary `String` on every invocation.
- **No `.collect::<Vec<_>>()` on splits** - iterate directly over the iterator.
  Collecting into a `Vec` allocates heap memory unnecessarily.
- **No `String::from(ch)` in escape loops** - use stack-allocated buffers.
  Converting a single character to a `String` is a heap allocation per
  character.
- **No per-request template re-parsing** - load the compiled protocol once at
  startup and reuse it for every request.
- **No per-request protocol decoding** - use `PreparedProtocol` or keep the same
  Node protocol `Buffer` alive across requests.

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

# Hydration-state projection and partial-state serialization
cargo bench -p microsoft-webui-handler --bench bootstrap_state_bench

# Prepared versus one-shot FFI startup cost
cargo bench -p microsoft-webui-ffi --bench prepared_protocol_bench

# Contact book end-to-end benchmark
cargo bench -p microsoft-webui --bench contact_book_bench

# Results with HTML reports
ls target/criterion/report/index.html
```

Each benchmark uses [Criterion.rs](https://github.com/bheisler/criterion.rs)
for statistical rigor - results include confidence intervals, outlier
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

Each hydrated component produces a `webui:hydrate:<tag>` measure entry (where
`<tag>` is the custom element tag name), making it straightforward to identify
slow components and optimize them individually.

## Learn More

- [SSR showdown source](https://github.com/microsoft/webui/tree/main/examples/integration/ssr-performance-showdown) - full benchmark harness and reproduction steps
- [Contact book benchmark](https://github.com/microsoft/webui/tree/main/crates/webui/benches) - real-world application benchmark
- [DESIGN.md](https://github.com/microsoft/webui/blob/main/DESIGN.md) - architectural performance principles
