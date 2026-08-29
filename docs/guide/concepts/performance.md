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

With a validated projection manifest, the initial page contains only top-level
`@observable` and `@attr` values needed by reachable authored components.
Template-only values and inactive routes do not enlarge startup state. Without
a manifest, WebUI preserves full state.

Representative final release-mode Criterion results on a 1 MiB state object:

| Workload | Before | After | Improvement |
|----------|-------:|------:|------------:|
| Initial render, full state vs three exact keys | 1.9034 ms | 1.6724 us | 1,138x |
| Partial route, legacy parse/materialize/reserialize vs borrowed projection | 6.6751 ms | 658.17 us | 10.14x |
| Partial route, borrowed full JSON vs borrowed projection | 1.0191 ms | 658.17 us | 35.4% less CPU |

The initial-state benchmark starts from an already parsed Rust value. Hosts
that accept JSON strings still pay JSON parsing cost. Loading `protocol.bin`
into a `Protocol` once avoids repeated protocol decoding, not state parsing.

The same current Contact Book protocol and state isolate payload impact:

| Initial route | Full strategy | Projected strategy | Reduction |
|---------------|--------------:|-------------------:|----------:|
| Contact Book home | 12,116 bytes | 37 bytes | 99.69% |
| Contact Book contacts | 12,116 bytes | 37 bytes | 99.69% |

The live Contact Book API omits an empty `searchQuery`, so its projected
`/contacts` response carries only `{"totalFavorites":5}` (20 bytes). Projection
never synthesizes keys that the request state did not provide.

Projection adds build-time work, not browser bundle code:

| Paired Contact Book build | Without adapter | With adapter | Cost |
|---------------------------|----------------:|-------------:|-----:|
| Clean esbuild build, median of 6 alternating pairs | 90.9 ms | 131.8 ms | +40.9 ms |
| No-change rebuild, median of 10 alternating pairs | 30.6 ms | 63.4 ms | +32.8 ms |
| Emitted browser files | 398,341 bytes | 398,341 bytes | byte-identical |

The production manifest was 9,753 bytes and added 450 bytes (1.62%) to the
27,804-byte protocol. A profiled docs build hashed all 50 graph modules but
parsed only 10 semantic modules; the complete adapter phase was 122 ms median.
These numbers are hardware-specific, but they bound the tradeoff: tens of
milliseconds during client builds in exchange for removing large-state
serialization from every request.

Every host should load `protocol.bin` once at startup:

- Rust: construct `Protocol`
- C: use `webui_protocol_create` and pass the handle to every protocol operation
- .NET: construct `Protocol` and reuse it with the handler
- WASM: construct the exported `Protocol`
- Node: construct `Protocol` with the bytes and plugin, then release or reuse
  the source `Buffer`

The dedicated FFI startup benchmark isolates protobuf decoding and index
construction from application rendering:

| Protocol size | Full prepare/request | Full reused | Partial prepare/request | Partial reused |
|---------------|---------------------:|------------:|------------------------:|---------------:|
| 100 components | 77.390 us | 0.522 us | 72.673 us | 1.214 us |
| 1,000 components | 716.67 us | 1.023 us | 790.70 us | 1.310 us |

Protocol reuse removes 98.3% to 99.86% of the isolated protocol startup cost.
The relative impact on a real request is smaller when template rendering or
state serialization dominates, but repeatedly decoding immutable protocol
bytes remains avoidable work.

Do not construct a new Node `Protocol` for every request. Keep one loaded
instance for the server lifetime.

**Browser state is client-facing.** Never put credentials, private tokens, or other secrets in state rendered to
the browser.

## Offscreen Work Reduction Showdown

The complete `<template w-render="lazy">` policy is intended for long
lists where most instances start outside the browser's relevant region. The
production Chromium matrix uses 30 timing/heap runs and five trace runs per
cell. "Staged ready" adds initial render presentation to hydration readiness
because the harness deliberately isolates those phases.

| Items | Eager staged ready | `w-render="lazy"` staged ready | Render ready reduction | Hydration CPU reduction | Initially hydrated | Total retained heap reduction |
|---:|---:|---:|---:|---:|---:|---:|
| 10 | 25.0 ms | 41.3 ms | 20% faster | 27% slower | 10 / 10 | 12% more |
| 100 | 24.7 ms | 42.5 ms | unchanged | 7% less | 13 / 100 | 2% less |
| 1,000 | 77.7 ms | 58.0 ms | 54% faster | 35% less | 13 / 1,000 | 44% less |

At 1,000 rows, the complete policy also reduced Chromium style CPU by 70%,
layout CPU by 65%, pre-paint CPU by 86%, paint CPU by 54%, and live layout
objects by 91%. The trace window ends before hydration and interactions, so
these figures isolate initial rendering. HTML parsing stayed at 3.3ms and the
DOM stayed at 15,002 nodes in both arms. That is expected: the policy preserves
SSR semantics and reduces rendering/hydration work, not parsing or DOM
construction. Raster CPU was 7.4ms versus 5.5ms in this run; Chromium already
avoids most raster work outside the viewport, so layout and pre-paint are the
more stable work-reduction signals.

Hydration-only `w-hydrate="lazy"` at 1,000 rows reduced hydration CPU from
12.4ms to 7.2ms and total retained heap from 1,480.5KiB to 811.3KiB, but staged
readiness was 83.8ms because it did not reduce layout and paint. This is why
`w-render="lazy"` is the recommended long-list policy and
`w-hydrate="lazy"` is the containment escape hatch.

The crossover sits between 100 and 1,000 rows. Below it, the observer round
trip costs more than the deferred work returns. Keep small, fully visible
groups eager. Eager is the default, so applications that do not opt in retain
the existing behavior and ship no coordinator code. The optional entry adds
1,446 gzip bytes.

Reproduce the matrix:

```bash
cd examples/integration/streaming-browser-bench
WEBUI_LAZY_HYDRATION_RUNS=30 \
WEBUI_LAZY_HYDRATION_TRACE_RUNS=5 \
pnpm test:lazy-hydration
```

## Why WebUI is Fast

Each layer of the architecture contributes to the overall performance profile:

- **Build-time compilation.** Template parsing, component discovery, and
  expression compilation all happen once during `webui build` (or on the fly
  with `webui serve` in development). At runtime, the server only performs
  state interpolation against a pre-compiled binary protocol - no syntax
  parsing, no AST walking.

- **Bundler-proven state projection.** The application bundler records exact
  decorated state ownership and emitted chunk membership. WebUI validates that
  sidecar once, stores compact surfaces in `protocol.bin`, and projects request
  state without running TypeScript analysis in Rust.

- **Protocol Buffers.** The handler consumes a compact binary payload instead
  of parsing template syntax. Host `Protocol` APIs decode the protocol and
  build deterministic indices once at startup rather than repeating that work
  per request.

- **Runtime-discovered streaming.** The continuation VM walks only the selected
  entry, component, condition, and route path. It is iterative and keeps bounded
  frames, projected parent keys, lexical locals, static-occurrence keys, and
  generated component spans instead of cloning full state for every boundary or
  prebuilding a request plan. A repeat body cannot reach a boundary, so each
  `<for>` finishes inside its current step and no repeat iterator survives a
  host call. The synchronous `render_streaming` path borrows one state and uses
  one prepared render context for the complete response. Ordered range records
  reuse the exact prior projection by sequence and serialize only top-level
  additions or replacements when the current projection is a proven superset
  under the same state revision. Boundary-free
  fragment records are skipped through a build-time `contains_boundary` bit.
  Capture and projection scratch buffers retain capacity across checkpoints.
- **Bounded browser activation.** Each checkpoint or generated span completion
  walks one root-local marker range, including open shadow roots, and removes
  its scaffolding after commit. Final occurrences retain no root list.
  Updatable occurrences retain only successfully activated roots until
  terminal. The valid path has no `MutationObserver`, polling loop, or
  document-wide selector; only fatal cleanup may perform one bounded sweep.
- **Host-owned backpressure.** `StreamingWriter` uses a bounded `tokio::mpsc`
  channel, reusable `ChunkPool`, and configurable flush deadline. Owned
  `StreamingSession` calls return one byte chunk for `start`, `resume`,
  `advance`, or `update`, so Node, WASM, Python, C, and .NET hosts write through
  their native backpressure APIs. A boundary-only `resume` can flush immediately;
  `advance` carries the following parent and tail bytes. Rust hosts can transfer
  freshly loaded values by passing owned `Value`s to `start` and `resume`, or
  call `resume_current` when the retained snapshot is unchanged. `webui serve
  --api-port` uses a capacity-one version-2 control channel over the same
  session.
  Hosts must also bound concurrent blocking renders before calling
  `spawn_blocking`; channel backpressure bounds bytes after a task starts, not
  the runtime's queued blocking-task count. Reject saturation before spawning
  so retained request state stays bounded.
  Intermediaries can still buffer the response, so production deployments must
  configure and verify their full delivery path.

- **No JavaScript runtime.** There is no V8, no garbage collector pauses, and
  no JIT warmup. The hot path is pure compiled Rust with predictable, low-
  latency execution.

- **Targeted updates.** On the client side, path-indexed binding updates touch
  only the affected DOM nodes - not entire subtrees. This keeps hydration and
  reactive updates fast even in large documents.

### Progressive streaming cost profile

The progressive path discovers runtime occurrences, preserves continuation
state, and completes generated component spans. A fixed entry-boundary
benchmark provides a lower-feature baseline for these interleaved release-mode
measurements:

| Boundaries | Fixed model | Runtime-discovered model |
|-----------:|------------:|-------------------------:|
| 1 | 2.20 us | 2.57 us |
| 3 | 4.25 us | 4.58 us |
| 10 | 9.95 us | 11.05 us |
| 100 | 76.1 us | 98.3 us |

The ordinary renderer remains effectively unchanged in the same comparison
(732 ns versus 741 ns). A separate large-state benchmark verifies that frozen
state is projected once per response: optimization reduced an eight-boundary
render from 354.6 us to 114.1 us.

The optional browser coordinator is also measured independently from the
always-shipped framework entry:

| Production bundle | Minified | Gzip |
|-------------------|---------:|-----:|
| Ordinary framework entry | 60,528 bytes | 18,993 bytes |
| Streaming-only increment | 16,652 bytes | 5,928 bytes |

The hydration matrix enforces absolute ordinary and incremental limits with
4-5% headroom, so moving optional streaming code into the default entry cannot
hide behind subtraction. These figures are workload and machine specific; use
`examples/integration/streaming-browser-bench` and
`streaming_hydration_bench` for changes to either hot path.

## Light DOM vs Shadow DOM

Neither mode is universally faster. Light removes ShadowRoot and per-instance
stylesheet objects, enables aggressive CSS bundling, and can improve SSR
throughput and document completion. Global Light CSS shares one large CSS tree,
so broad or frequent class/attribute invalidations may cost more than native
Shadow isolation. Shadow pays more parsing and stylesheet-instance overhead but
can recalculate styles substantially faster in CSS-heavy repeated component
trees.

### When to Use Each

**Light DOM** - build with `--dom light` when:
- Components are mostly static or moderately styled
- Components benefit from normal CSS inheritance
- CSS request/stylesheet consolidation matters
- Global CSS composition is intentional
- Native slot composition is not required

**Shadow DOM** - use when:
- You need native `<slot>` composition
- Components run in unknown host pages
- A native Shadow boundary is an explicit requirement
- Components have large stylesheets, many repeated instances, or frequent host
  class/attribute changes

### Authoring Shadow DOM

Shadow is the default fallback for unwrapped components. `--dom light` makes
unwrapped components Light. A sole bare top-level `<template>` is also an
explicit Light wrapper; `<template shadowrootmode="open">` explicitly remains
Shadow in either build mode.
Invalid or closed wrappers fail every build; `<slot>` fails only when the
effective component mode is Light.

Keep authoring ordinary paired CSS in both modes. Light CSS remains authored
global CSS and rejects Shadow-only selectors such as `:host`; Shadow CSS keeps
native selector semantics.

Component resources can use the router's template inventory, but bundled chunks
have distinct identities and are never inferred from component bits. Chunk
metadata lists the component resources it covers, so claiming one SSR resource
also prevents each Light descendant from reinstalling a fallback stylesheet.

Progressive streaming keeps a separate request-local style resource inventory.
Each closure and CSS definition is serialized at most once per response, even
when later boundaries reuse a component or a resource arrived transitively
through an earlier closure.

In a Light build, add open wrappers to slot, encapsulation, or CSS-heavy
frequently restyled components.

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
- **No per-request protocol decoding** - construct one `Protocol` and share it
  across renders.

## Running Benchmarks

Use the built-in benchmark suite to measure performance on your own hardware:

```bash
# All Rust Criterion benchmarks
cargo xtask bench all

# Individual crate benchmarks via xtask
cargo xtask bench parser
cargo xtask bench handler
cargo xtask bench expressions
cargo xtask bench protocol
cargo xtask bench state

# Hydration-state projection and partial-state serialization
cargo bench -p microsoft-webui-handler --bench bootstrap_state_bench

# Loaded versus per-request FFI protocol startup cost
cargo bench -p microsoft-webui-ffi --bench protocol_bench

# Contact book end-to-end benchmark
cargo bench -p microsoft-webui --bench contact_book_bench

# Public Node API across the real V8/N-API boundary
cargo xtask bench node-addon

# Results with HTML reports
ls target/criterion/report/index.html
```

Rust microbenchmarks use
[Criterion.rs](https://github.com/bheisler/criterion.rs) for confidence
intervals, outlier detection, and comparison against previous runs. The Node
addon benchmark uses `process.hrtime.bigint()` inside a real Node process and
supports the same named before/after baseline workflow through `cargo xtask`.

## Measuring Hydration Performance

WebUI emits a one-shot `webui:hydration-complete` event after the startup
hydration cohort completes. Visibility-deferred lazy components do not hold this
event open indefinitely or redispatch it when they activate later. Use
`hydratedCallback()` for instance-specific readiness and the Performance API to
inspect hydration timing:

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
- [Node addon benchmark](https://github.com/microsoft/webui/tree/main/examples/integration/node-addon-bench) - V8/N-API boundary benchmark
- [Browser hydration matrices](https://github.com/microsoft/webui/tree/main/examples/integration/streaming-browser-bench) - streaming and component-level lazy hydration
- [DESIGN.md](https://github.com/microsoft/webui/blob/main/DESIGN.md) - architectural performance principles
