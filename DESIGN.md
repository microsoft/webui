# WebUI architecture and specification

## Purpose

WebUI is a high-performance server-side rendering framework for building interactive web applications without a JavaScript runtime on the server. It compiles HTML, CSS, component metadata, routing, and state projection into a compact protocol. Hosts render that protocol with request state and stream HTML to browsers. The client hydrates only the Web Components that need interactivity.

This document is the architecture and behavioral specification for rebuilding WebUI. It records durable contracts, data shapes, invariants, and cross-layer decisions. It is not a change log, bug diary, code tour, or exhaustive implementation listing. Put short local rationale in source comments, public usage in `docs/`, and release or regression notes in PRs.

## Design goals

- **Server runtime independence.** Rendering must work from Rust and other host languages through stable protocol and FFI surfaces.
- **Build-time work over request-time work.** Parsing, dependency discovery, projection analysis, CSS ownership, and metadata construction happen during build whenever possible.
- **Low memory and predictable speed.** Hot paths use iterative traversal, buffer reuse, zero-copy borrowing where possible, bounded state, and deterministic serialization.
- **Progressive interactivity.** Static HTML is usable immediately; dynamic islands hydrate when their scripts and state are available.
- **Deterministic output.** Build output, protocol ordering, route matching, CSS closure order, and diagnostic codes must be stable across machines and process runs.
- **Actionable failure.** Invalid templates, state, routes, assets, and host inputs fail with typed errors and diagnostic help instead of panics or silent defaults.

## Non-goals

- Running application JavaScript during server rendering.
- Supporting arbitrary client framework runtime behavior in the server renderer.
- Treating JSON as the production wire format. JSON exists for inspection, debug output, and tests; protobuf is the production protocol.
- Hiding malformed input behind compatibility fallbacks.

## System layers

```text
source files
  -> parser and discovery
  -> protocol and projection manifest
  -> handler render/session API
  -> host integration or FFI binding
  -> browser runtime hydration
```

| Layer | Responsibility | Primary crates or packages |
|-------|----------------|----------------------------|
| Protocol | Stable serializable representation of templates, metadata, routes, CSS, projection, and streaming markers | `webui-protocol` |
| Parser and discovery | Convert author files and package metadata into protocol fragments and component metadata | `webui-parser`, `webui-discovery`, `webui-tokens` |
| Expressions and state | Resolve state paths and evaluate template conditions consistently | `webui-state`, `webui-expressions` |
| Handler | Render protocol with request state into HTML, partial responses, or streaming sessions | `webui-handler`, `webui` |
| Client runtime | Hydrate compiled templates, route islands, lazy work, and streaming boundaries | `packages/webui-framework`, `packages/webui-router` |
| Tooling and distribution | CLI, Node, WASM, Python, .NET, desktop, and FFI packaging | `webui-cli`, `webui-node`, `webui-wasm`, `webui-python`, `webui-ffi`, `webui-desktop` |

## Global invariants

- Core traversal algorithms are iterative, not recursive.
- Core parsing and rendering avoid regular expressions in hot logic.
- Library crates return `Result` for recoverable failures and never panic on user input.
- Errors intended for users include a stable code, source location when available, and actionable help.
- Public APIs expose the minimum required surface and document every exported type or function.
- Source files covered by the license check start with the Microsoft MIT header.
- Workspace dependencies are versioned only in the root `Cargo.toml`.
- Release builds use `panic = "abort"`, so FFI and host boundaries must never rely on unwinding.

## Protocol model

The protocol is generated from `proto/webui.proto` with `prost`. The protobuf schema is the canonical binary contract. Rust types mirror generated protobuf shapes rather than a separate domain model.

A protocol contains:

- fragment records keyed by entry or component template identifier;
- component metadata keyed by tag name;
- route metadata;
- CSS delivery strategy and style resources;
- state projection metadata;
- module preload and asset information;
- optional build outputs such as bundled style chunks and projection manifests.

### Fragment graph

Fragments describe renderable structure. The handler walks fragment records with a stack, not recursion.

| Fragment | Meaning |
|----------|---------|
| Raw | Literal HTML bytes produced by the parser or plugin |
| Component | Reference to another fragment record with component metadata |
| Attribute | Static or bound attribute emission |
| Signal | Compiler-owned insertion point or author state signal |
| If | Conditional branch controlled by an expression |
| For | Repeated block controlled by state iteration |
| Route | Server-resolved route branch |
| Outlet | Route child insertion point |
| Plugin | Plugin-owned fragment payload |
| Boundary | Streaming boundary declaration marker |

Fragment ordering is observable. Any transformation that reorders fragments must preserve the same rendered HTML, route semantics, CSS cascade order, and hydration metadata.

### Boundary declarations

A boundary is a typed declaration in the fragment graph. It is represented as paired start/end markers in the owner record, with a stable declaration ID and response-local runtime occurrence IDs. Ordinary rendering ignores the markers. Streaming uses them to suspend, resume, checkpoint, and continue without reparsing or cloning the whole document.

Boundaries may appear in entries, components, conditions, outlets, and selected routes. Boundaries reached from repeats are rejected because an unbounded number of runtime occurrences would defeat deterministic memory limits. Boundary keys are optional expressions used to identify repeated declarations reached through static callsites.

### Routes and outlets

Routes are resolved on the server. The client router consumes a server-produced route chain and diffs chains instead of re-matching paths. Nested routes render through `<outlet>` fragments. Only one outlet is supported per route level; duplicates are a build warning and later outlets at the same level are ignored.

Route matching must be deterministic. Parameterized, wildcard, exact, query, and nested route behavior must remain equivalent across Rust, Node, WASM, and browser integration surfaces that expose routing.

### Protocol evolution

- Additive fields must preserve behavior when absent.
- Removed field numbers and names must be reserved in the protobuf schema.
- Enum consumers must handle unknown or future values safely.
- New fields must justify decode cost and payload size.
- Changes cascade through handler, bindings, CLI inspection, and docs in the same PR.
- `DESIGN.md` records the durable contract, not implementation history.

## State and expression semantics

State is JSON-like data supplied by the host. Path resolution uses dot-separated segments. Missing paths render as empty content unless the operation explicitly requires an error. State lookup must avoid cloning large subtrees on read-only paths.

Expressions are parsed at build time and evaluated at render time against state and local scopes. Supported semantics include literals, path reads, comparison, boolean operations, and predicates used by template directives. Operators must behave consistently in parser validation, Rust handler evaluation, WASM, Node, and client projection surfaces.

For loops and conditions create local scopes. Scope save/restore is key-based: implementations must not clone entire scope maps per iteration.

## Parser and compiler

The parser turns HTML templates into protocol fragments and component metadata. It owns authoring validation, structural signals, plugin dispatch, CSS ownership, routing declarations, and component registration.

### HTML and component rules

- Components are identified by custom-element tag names.
- Component templates may use light DOM or a single declarative shadow root according to the component model.
- Template syntax is validated at build time when possible.
- Authored `<webui-hydrate>` is reserved and rejected.
- Compiler-owned signals use the WebUI namespace so authored state keys with similar names remain ordinary content.
- Structural `head_start`, `head_end`, `body_start`, and `body_end` signals are inserted around the real document boundaries.

### CSS strategy

CSS strategy is selected at build time and applies consistently across the protocol:

| Strategy | Contract |
|----------|----------|
| Link | Emit external stylesheet hrefs and preload metadata |
| Style | Inline CSS where the runtime owns the style resource |
| Module | Emit import maps or module-backed style resources for component CSS |

Component style closures preserve cascade-sensitive first-discovery order. Resource names are stable and deduplicated. CSS ownership follows the component and route tree, not incidental render order.

### Design tokens

Token extraction collects CSS custom property names and theme references at build time. Token resolution is deterministic and package-aware. Invalid token references produce diagnostics with suggestions when possible.

### Discovery and package resolution

External component discovery classifies source files, resolves npm packages, and caches results with all relevant inputs in the cache key. Discovery must cap file reads, reject unsafe paths, support workspace links, and avoid following arbitrary external symlinks without policy.

## Handler rendering

The handler renders a protocol with request state into HTML. It must be usable directly from Rust, through FFI, and through higher-level packages.

### Core API contract

The handler accepts:

- protocol bytes or an already-loaded protocol;
- render options such as entry ID, request path, nonce, and trusted injection strings;
- request state.

It produces HTML or streaming steps through a writer abstraction. Writers receive many small writes, so handler code must minimize temporary strings and let specialized writers coalesce output.

### Writer contract

`ResponseWriter` writes HTML pieces and finalizes output. Streaming requires `FlushWriter`, which adds an explicit `flush()` boundary. A streaming render must reject a writer that cannot flush rather than silently buffering the response.

Per-render head and body injections are emitted at structural parser signals, not by scanning bytes for closing tags. Hosts are responsible for passing trusted, already escaped injection HTML.

### State injection channel

Reserved `$webui` state keys carry framework injection data such as head-end, body-start, and body-end snippets. The channel is consumed by the handler and stripped from application state projections so it cannot leak into component state.

### Handler plugins

Plugins own framework-specific rendering behavior while the core handler owns traversal, state, routing, and writer contracts. Plugin output must remain deterministic and must not introduce panics or hidden global state.

## Client runtime and hydration

The browser runtime hydrates server-rendered Web Components from compiled template metadata. It does not rebuild UI with imperative DOM construction. Author UI lives in templates and CSS; JavaScript is used for interactivity and browser APIs.

### Metadata contract

Component metadata provides enough information to:

- clone template DOM;
- bind attributes, text, and events;
- project initial state;
- load component styles;
- activate lazy, interaction, or streaming work policies.

Metadata is JSON-safe and versioned by the package contract. Missing or malformed metadata fails closed rather than partially hydrating an inconsistent tree.

### Hydration lifecycle

Hydration walks the DOM once per relevant range, removes generated scaffolding after use, and activates roots in parent-before-child order when required by the work policy. `hydratedCallback()` is once per instance readiness, not a global page barrier.

Lazy hydration may be triggered by visibility, interaction, or explicit policy. The runtime must preserve targeted updates and avoid per-update array or DOM allocations in hot paths.

## Progressive streaming hydration

Streaming lets hosts flush usable shell HTML, suspend at boundaries, later resume them with state, and notify the browser when the response is complete.

### Streaming wire shape

Each streamed record is emitted as:

```html
<script type="application/json" data-webui-boundary>[sequence, kind, target, payload]</script><webui-hydrate></webui-hydrate>
```

`sequence` is response-local and strictly increasing. `kind` identifies checkpoint, updatable checkpoint, state update, span completion, or terminal. `target` identifies the boundary or span namespace for that kind. `payload` carries state and metadata deltas.

The script and sentinel are an inseparable pair. The sentinel upgrades, finds the immediately preceding payload script, enqueues the record, and the coordinator removes both after commit.

### Streaming placement contract

All boundary payload/sentinel pairs, including the terminal record, are part of the document body. The terminal record is emitted at the structural `body_end` hook after body-end injections and before the raw closing document tail. The final flush coalesces the terminal record with that raw tail.

### Streaming lifecycle

1. `start` writes the shell until the first boundary or terminal.
2. `resume` commits one pending boundary and flushes that checkpoint.
3. `advance` writes ordinary parent bytes until the next boundary or terminal.
4. `update` may patch a committed updatable boundary before terminal.
5. The terminal record completes the response-scoped lifecycle.

The browser coordinator is document-scoped and FIFO. It commits records in sequence, validates target ordering, applies metadata/state deltas, activates roots, records performance marks, and releases scaffolding. `webui:hydration-complete` fires only after the terminal record and all pending work settle successfully.

Malformed records, sequence gaps, stale targets, missing markers, duplicate live keys, truncated streams, and unsupported limits fail closed. Completion must not fire on a failed stream.

### Streaming limits

Server and browser limits bound continuation depth, queued records, boundary occurrences, updatable occurrences, generated spans, retained roots, marker scans, and frozen state keys. These limits are part of the safety contract: exceeding them is an explicit error, not silent truncation.

## Bundler-neutral projection compiler

The projection compiler determines which application state keys are required by each component, route, and navigation surface. Its output lets the handler ship only required state to the browser.

### Build order

1. Discover components and entry files.
2. Build the JavaScript and CSS module graph through an adapter.
3. Analyze TypeScript decorators, component registration, routes, and state access.
4. Emit deterministic protocol metadata and projection manifests.
5. Validate output membership, stale artifacts, and security constraints.

### Adapter contract

Adapters expose a normalized module graph independent of the bundler. The esbuild adapter is the reference adapter, but the projection compiler contract is bundler-neutral. Adapters must provide deterministic ordering, source identity, output identity, and enough metadata to map physical outputs to served URLs.

### Projection safety

Unknown dynamic access falls back to complete state for the affected surface. Exact known access may use key projection. The compiler must prefer correctness over smaller state when analysis is uncertain.

Projection diagnostics are stable and categorized by compiler, peer dependency, manifest, build validation, and security/resource failures.

## Routing and navigation runtime

The server owns route matching and returns the selected chain. The client router uses that chain to mount only changed levels during soft navigation. Parent components remain mounted when navigating between sibling routes. Route component JavaScript may be lazy-loaded through dynamic import.

Route and navigation state projection must match server render semantics. Query strings, dotted path segments, encoded characters, and nested outlets are route data unless the protocol explicitly marks an asset request.

## CLI and developer tooling

The CLI builds, inspects, serves, packages, and scaffolds WebUI applications. CLI commands may use `anyhow`; library crates use typed errors. CLI presentation may add color with `console::style()`, but machine-readable output and library errors stay color-free.

`webui inspect` and debug output may use JSON to help humans and tools inspect protocol contents. Production hosts should use protobuf bytes.

## Host and language integrations

### Rust

Rust hosts can use loaded protocols, handler render APIs, streaming sessions, and the `webui::streaming::StreamingWriter`. Streaming hosts must honor flush boundaries and propagate writer failures immediately.

### FFI

The C ABI exposes opaque handles, byte buffers, errors, and streaming session functions. Every exported function validates foreign inputs, catches panics, sets last-error state, and returns a safe sentinel on failure. Header files are generated and must stay in sync with exported signatures.

### WASM, Node, Python, and .NET

Language bindings expose the same logical operations as the Rust API: load protocol, render, inspect errors, and operate streaming sessions where supported. Binding differences may reflect host language conventions, but result shapes and error semantics must remain equivalent.

### Desktop

The desktop runtime packages WebUI applications with native windows, local protocols, route/state providers, typed IPC, and platform-specific window behavior. Desktop IPC is bounded, proofed per document, and explicit about cancellation, retirement, and permissions. Packaged hosts deny development-only capabilities unless explicitly allowed by policy.

## Error and diagnostic policy

- Parser and build authoring errors use structured diagnostics with stable codes.
- Handler and runtime errors use typed error enums.
- Diagnostics include enough source context and help for authors to fix the problem.
- Color and formatting are presentation concerns in CLI layers only.
- Cold error builders should stay out of hot-path layout.
- Tests should assert stable codes and structured fields instead of fragile prose when available.

## Performance and memory policy

- Prefer borrowed state, slices, and `Cow` over owned clones.
- Preallocate buffers when size is known or estimable.
- Reuse scratch buffers across checkpoints and render steps.
- Do not clone full JSON state, protocols, route trees, or scope maps for read-only lookup.
- Avoid per-update arrays, maps, closures, DOM queries, and retained temporary element properties in client hot paths.
- Keep generated JavaScript small because parse and compile memory count against the client budget.
- Benchmark performance-sensitive changes and report meaningful deltas for `perf:` PRs.

## Security and resource policy

- Paths are normalized and checked before file access.
- Discovery and build inputs have size bounds.
- External URLs, symlinks, package metadata, and generated served URLs are validated before inclusion.
- CSP nonces are reflected consistently on framework-generated inline scripts.
- HTML injection APIs accept trusted HTML only; callers must escape untrusted content before passing it.
- Transport and IPC queues are bounded so slow or malicious clients cannot force unbounded memory growth.

## Documentation boundary

- `DESIGN.md` records architecture, durable contracts, protocol shapes, and rewrite-level decisions.
- `docs/` explains supported public APIs and author workflows for application developers.
- `docs/ai.md` is the compact authoring reference for code-generation agents.
- Source comments explain local implementation choices that are not architectural contracts.
- PR descriptions and tests carry regression history.

## Quality gate

Before commit or push, run:

```bash
cargo xtask check
```

The gate runs license headers, formatting, clippy, dependency audit, tests, builds, WASM builds, example builds, benchmark validation, and docs validation. On Windows, use process-local `TEMP` and `TMP` on the worktree drive when projection or docs output must share a filesystem root.
