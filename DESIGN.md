# WebUI Architecture

WebUI compiles HTML, CSS, and component declarations into a compact render
program. A Rust handler combines that program with request state to produce
server-rendered HTML without running application JavaScript. Browser code is
optional and hydrates only the components that need interactivity.

This is the **living architectural map** for rebuilding WebUI in another
technology. It describes responsibilities, data flow, ownership, and decisions
whose loss would change the system. Change it when those decisions change, not
for every new field, method, diagnostic, optimization, or bug fix. The exact
wire schemas, supported APIs, authoring syntax, and executable edge cases live
in the sources and references listed at the end.

## Design constraints

- **Compile once, render many times.** Discover dependencies, parse templates,
  choose component policies, and plan CSS and client state at build time. A
  request must not parse templates, analyze JavaScript, or rebuild dependency
  graphs.
- **Render without a JavaScript runtime.** The server consumes a binary
  protocol and JSON state; JavaScript is for browser interactions and optional
  build-time analysis, never for SSR.
- **Preserve ordinary HTML.** SSR is useful before hydration. Components can
  be HTML-only; authored browser behavior is an opt-in island rather than an
  application-wide prerequisite.
- **Bound work and memory.** Core scanners and render traversals avoid regex
  and recursion. Consolidate static fragments and writes, borrow state where
  possible, precompute reusable indexes, and impose limits on untrusted input,
  queued work, and retained streaming state.
- **Fail explicitly.** Invalid authoring input and incompatible artifacts are
  actionable errors, not panics, empty output, or silent compatibility guesses.
  Once streamed bytes are committed, an error cannot become a different HTTP
  response.

## System shape

```text
HTML/CSS + components       optional application JS/TS
          |                            |
  discovery + parser          bundler + projection adapter
          |                            |
          +------ build-time proof -----+
                         |
               protocol.bin + assets
                         |
     JSON state -> immutable Protocol -> Rust handler -> HTML / route partials
                                                     |
                                  optional browser framework + router
```

The **compiler** (`webui`, `webui-discovery`, `webui-parser`,
`webui-protocol`, `webui-tokens`) owns source interpretation and the serialized
artifact. The **handler** (`webui-handler`, `webui-state`,
`webui-expressions`) owns request-scoped evaluation and output; it does not
know how to find source files. The **browser** (`@microsoft/webui-framework`
and, independently, `@microsoft/webui-router`) consumes compiler output and
server HTML rather than compiling templates again. CLI, desktop, and language
bindings are hosts around the same protocol and handler.

These are separable boundaries: an application can render without a router or
browser runtime, and a host can load `protocol.bin` without shipping the
compiler. Only the selected optional browser entry points should reach an
application's JavaScript bundle.

## Compilation and artifact ownership

### Source discovery and parsing

`webui-discovery` turns application directories, local paths, and npm packages
into component sources. Discovery plugins may interpret different layouts
(notably native WebUI and FAST), but registration, name validation, duplicate
detection, and parsing remain shared. Native filename-based discovery uses a
matching `.ts` or `.js` sibling to distinguish authored client behavior from
scriptless HTML; other plugins may supply their own ownership evidence.
Filesystem and package resolution are build-time concerns; paths from packages
must be validated before they are opened.

`webui-parser` scans HTML and CSS deterministically and compiles entry pages
and reusable components into fragment records. Static HTML is coalesced; dynamic
text, attributes, conditions, loops, routes, outlets, and streaming boundaries
become typed instructions or references. `<if>`, `<for>`, `<route>`,
`<outlet>`, and `<boundary>` are compile-time directives, not browser elements.
`{{value}}` is escaped on output; `{{{value}}}` is trusted raw output. The
parser rejects invalid structure and unsupported syntax with source-located
diagnostics, rather than trying to repair arbitrary browser HTML.

Component templates may share named loop bodies without expanding them at each
callsite. Structural recursion through component templates is invalid; finite
data-driven loop reuse is supported. Lexical scopes are compiled so the
handler can evaluate a loop item's fields alongside global state without
copying the state tree.

Framework-specific source conversion and hydration metadata belong to paired
**discovery, parser, and handler plugins**. The protocol carries opaque plugin
bytes; core rendering does not inspect a framework's element metadata.
Selecting no handler plugin yields plain SSR HTML. WebUI and FAST use the same
fragment graph but different client payloads and marker conventions.

### The binary protocol

`WebUIProtocol` is the generated protobuf build/wire model. It contains:

- A map of entry/component fragment records, with typed operations for raw
  content, bindings, attributes, conditions, loops, components, routes,
  outlets, plugin data, and boundaries.
- Per-component client-template data, effective DOM/work policy, and
  hydration/navigation state surfaces.
- Build-wide token inventory, CSS delivery policy, ordered style resources,
  optional preload hints, and other precomputed render metadata.

References connect records without copying component bodies into every use.
Routes are nested in the fragment graph, not maintained in a parallel global
route registry. Streaming boundary declarations sit inside their owner's
record; response-local occurrences are discovered only when rendering that
path. The on-disk format is protobuf, not JSON. `webui inspect` may expose JSON
for debugging, but hosts render the binary artifact. Schema evolution must
preserve or explicitly change producer/consumer compatibility; removed field
numbers are reserved. `crates/webui-protocol/proto/webui.proto` is the exact
schema, including field numbers and enum values.

At startup, the handler wraps the decoded build model in an immutable,
shareable `Protocol`. This runtime object builds indexes and caches that do
not belong on the serialized message. A host loads it once, shares it across
requests, and creates separate render contexts for state, route, nonce,
inventory, writer, and plugin instance. No render should decode or re-index
the protocol per request.

### CSS and themes

The build determines each component's **effective DOM mode**. Unwrapped
components default to an open declarative Shadow root; a sole bare
`<template>` opts into Light DOM, a sole authored open
`<template shadowrootmode="open">` stays Shadow, and the build's Light
fallback applies only where no root mode was authored. Light CSS is genuine
global CSS in its containing tree, not automatically selector-scoped. A
ShadowRoot is a CSS ownership boundary.

For each Document or ShadowRoot entry, compilation records a style closure in
cascade order. Light descendants contribute to their parent's CSS tree;
Shadow descendants start a new tree. Route-specific closures follow the
selected route. The handler and browser install each resource once per CSS tree
from this build-time plan, not from request-time graph traversal. The CSS
delivery choice is independent of ownership: external links, inline styles,
or CSS modules with an SSR style fallback. Optional bundling may group styles
only when all consuming trees require the same adjacent rules in the same
order; it cannot change cascade semantics.

The CSS scanner also records custom-property uses and unresolved fallback
chains. `webui-tokens` validates optional themes and resolves needed token
values at build/serve time. The protocol retains an ordered, deduplicated
token inventory for hosts, rather than making each request scan stylesheets.
Compiler-produced modulepreload hints similarly describe critical static
imports ahead of time; deferred imports must not be promoted accidentally.

### Optional state projection

Without a projection manifest, initial hydration uses full state and unknown
scripted navigation surfaces retain full state for correctness. The WebUI
projection compiler is a **build-only** TypeScript analysis in
`@microsoft/webui/projection.js`, not part of the Rust render path or the
package's root runtime import. A bundler adapter supplies its already
resolved module graph and final output membership. The compiler traces
WebUI-authored component definitions and observable/attribute properties,
including inherited properties, and emits exact per-component state keys.
It never infers a key set from raw output text or reruns module resolution.

One bundler build produces a deterministic, versioned JSON manifest; one WebUI
build validates it and produces `protocol.bin`. The manifest binds source and
output identities to their exact bytes so stale or conflicting fragments
cannot silently provide incorrect projection data. Manifests may be merged
only when component ownership is unambiguous. When a WebUI build opts into
manifests, every compiled scripted component must have exact coverage.
Unproven analysis fails the build; without manifests, unknown surfaces
conservatively select all state. Scriptless components require no JavaScript
analysis. The handler never opens manifests or bundles at request time.

Initial hydration and partial navigation have separate state surfaces.
`None` means proven empty, `Keys` means an exact set, and `All` means full
state is required. Initial state includes proven authored observable/attribute
properties; navigation additionally includes compiled template roots.
Reachability follows the active route but conservatively includes conditional
and loop branches it might render. Projection is a payload/performance
optimization, **not a secrecy boundary**: any browser render state may be
sent to the client. Never put secrets in it.

## Request-time rendering

`webui-handler` walks the selected fragment graph into a writer, resolving
bindings from JSON request state. Global state is available throughout; loops
introduce lexical item scopes, and component bodies receive the closest loop
item plus global state. Dotted paths and `.length` are supported; missing
simple identifiers are falsy and ordinary missing text bindings render empty.
Conditions are parsed at build time and evaluated without a JavaScript runtime.
Server and browser conditions share an intended model, but empty arrays and
objects are currently falsy on the server and truthy in compiled JavaScript;
templates must test a length or field instead of the collection itself.
Text and attributes are escaped by default. Raw bindings and per-render
injections are **trusted HTML**; their host owns input validation/escaping.

Authoring and protocol failures carry structured, stable diagnostic codes,
source locations, and actionable help. These are plain data across Rust and
language boundaries; the CLI owns terminal coloring and JSON presentation.
Runtime input, writer, and transport failures stay distinguishable so hosts
can choose the appropriate response and log failures after output commits.

The handler writes response metadata at structural document boundaries
identified by the compiler, never by scanning rendered HTML for closing tags.
For non-streaming WebUI pages, inert `#webui-data` contains the startup route
chain, inventory, styles, templates, and appropriately projected state.
Browser-executable condition closures are carried separately from JSON-safe
template metadata. Server output is useful without these extras when no
client plugin is selected.

Rendering targets a `ResponseWriter`, allowing buffered HTML or bounded,
backpressured transport writers. Writer failures and disconnects propagate;
the handler does not finish rendering into an abandoned channel. Host entry
points may render a whole document, a component fragment, a route partial,
or a host-paced streaming session from the same loaded protocol.

### Server-authoritative routing

Nested `<route>` declarations and a component's `<outlet>` define the route
tree. The server selects the active chain against the request path, preferring
more specific matches and declaration order for ties. Path parameters and an
explicit query-parameter allowlist supply route data; arbitrary query
parameters never become component attributes. Hidden sibling route
placeholders keep the original declaration order without rendering their
inactive content. An outlet inserts the matched child at the parent's chosen
location.

For soft navigation, the server returns the authoritative route chain and
projected state, filters templates against the client's component inventory,
and supplies separately tracked style resources required by the destination.
The router registers those resources, waits for their readiness, and reconciles
changed route levels while preserving
unchanged parents. It may fall back to document navigation when client-side
mounting cannot complete. Partial responses can be complete JSON or an NDJSON
pair that sends chain/templates first and state later; neither is the
progressive HTML streaming protocol. Cache tags, mutation invalidation,
pending/error components, and speculative preloading are optional router
features, not dependencies of the SSR handler.

### Optional component assets

Builds without route directives may request stand-alone component asset roots.
The compiler computes each root's conservative template/style dependencies
and emits reusable shared modules for overlapping closures rather than
duplicating payloads. The entry bundle retains ownership of entry-reachable
components. An asset declares its prerequisites and imports; the framework
validates coverage before registering any payload. Each CSS tree still gets
the required ordered style closure, including when a Shadow root is created
later. Neither a static asset module nor the handler assumes the browser's
current template inventory at build time.

## Browser hydration

The WebUI parser also compiles component templates into static, marker-free
client HTML plus compact binding, block, event, and state-path metadata.
Server HTML and client metadata come from the same source. SSR adds
compiler-owned markers where needed to align bindings and delimit dynamic
ranges. Hydration wires those bindings to the existing DOM; it does not
reinterpret authoring syntax,
re-render the first paint, or do a document-wide search per binding.
Client-created components clone compiled static HTML and fill the recorded
slots. Raw HTML ranges retain explicit ownership for later replacement.

`WebUIElement` is the authored behavior layer. Compiler-owned scriptless hosts
need no application module and can stay dormant until state or client creation
requires activation. Component work policies may defer SSR hydration to
visibility or interaction and may independently let the browser defer
offscreen rendering. The HTML remains present. A parent must hydrate before
ordinary descendants can mutate its SSR markers; successful first activation
calls the authored hydration lifecycle once. Optional coordinators are loaded
only when the application imports their entry points.

The framework treats compiled HTML and metadata as trusted output of its own
compiler, not as arbitrary user HTML. Trusted Types support protects compiler
sinks when available; it is not a sanitizer for triple-brace values or a
substitute for a host's CSP and escaping policy. SSR markup, markers, and
metadata must be produced by compatible compiler/handler versions.

## Progressive Streaming Hydration

Progressive HTML streaming is an explicit response mode, separate from ordinary
rendering and JSON/NDJSON navigation. Authors place `<boundary>` around
independently paced content in an entry or reusable component. A declaration
can be reached through the selected route or condition, but boundaries cannot
nest or execute inside a repeat. A repeat *inside* one boundary is allowed.
Multiple simultaneous occurrences of one declaration require stable keys.
The compiler rejects placements the HTML parser would move or that cross an
incomplete hydration scope.

The host drives one response session: `start` writes up to the next occurrence,
`resume` writes and flushes only that occurrence, and `advance` writes the
following parent/tail bytes until the next occurrence or terminal. An
updatable occurrence can receive state-only `update` records while the
response remains open. A session retains its continuation and a bounded
projection of parent state; it does not recompute the document or clone the
entire state tree on every resume. Calls out of order fail before writing.
After a transport failure the committed prefix cannot be rewound.

Records are ordered and response-local. Each checkpoint brings the additive
template, style, inventory, route, and state information needed to activate
its range after earlier records; the response ends with exactly one terminal.
Generated component spans let a completed child hydrate before an unfinished
parent without opening an arbitrary descendant through the normal parent
barrier. The browser coordinator commits complete ranges in order, hydrates
parent-first within each eligible range, and retains roots only when updates
are allowed. Updates use the same reactive state path; they never replace
markup or run hydration again. Malformed, stale, truncated, or over-budget
streams fail closed and release their scaffolding.

The coordinator is an explicit application import
(`@microsoft/webui-framework/streaming.js`), not globally injected into
framework modules. A stream-aware host owns pacing, flushing, backpressure,
and cancellation. Rust can write directly through a flush-capable writer;
language bindings return independently writable byte segments. Normal
non-streaming output does not acquire streaming markers or state.

## Host surfaces and deployment

| Surface | Responsibility |
| --- | --- |
| `webui` Rust library and CLI | Build/inspect protocols, render requests, and serve development apps. Watch rebuilds replace the served artifact only after a successful build. |
| `@microsoft/webui` | Node build API and native-addon runtime; its projection subpath is build-only. Browser framework and router are separate packages. |
| C FFI, .NET, Python, WASM | Load a protocol once and expose the same logical rendering, partial, token, and streaming capabilities with host-native ownership and errors. WASM can also build protocols when its parser feature is selected. |
| `webui-press` | Convert Markdown/site templates to WebUI page inputs and share a validated client projection across page builds. |
| `webui-desktop` | Package the compiled artifact and run it through a native webview and Rust host without an embedded HTTP server. |

FFI uses opaque handles and explicit ownership; .NET wraps those lifetimes in
safe handles, Python uses a direct PyO3 binding, and Node uses a native addon.
Owned streaming sessions are single-driver even when loaded protocols are
shared across concurrent requests. Host bindings must preserve the same
render and state semantics without silently falling back to a different
engine.

### Desktop boundary

The desktop shell uses WebView2 on Windows, WKWebView on macOS, and
WebKitGTK on Linux. One platform-neutral frame owns the compiled protocol,
window, state providers, assets, and capabilities; OS-specific adapters own
native lifecycle and FFI. Source builds and immutable bundles feed the same
custom-protocol request dispatcher. Rust providers can supply route state and
application API responses, so browser routing uses the same authoritative
protocol as server deployments. The desktop CLI invokes a separate sidecar,
keeping native webview dependencies out of the default CLI.

Optional application IPC is distinct from resource requests and window-control
messages. Proto3 application contracts generate typed Rust and TypeScript
payload codecs; an application must explicitly grant methods to the renderer.
The **framework envelope** uses a bounded fixed-layout binary codec shared by
the Rust host and its packaged renderer, not the application protobuf schema.
Admission is tied to the current native-verified main document; navigation
revokes the previous session. Calls, notifications, cancellation, deadlines,
worker capacity, and transport memory are bounded. Native UI callbacks do not
execute application handlers. Disabled IPC adds no active connection or
workers to a plain rendering app.

## Evolution and sources of truth

An architectural change updates this document when it changes a subsystem's
responsibility, a build/runtime boundary, a cross-layer invariant, or a
technology choice required to reproduce the design. Keep the description
conceptual and revise existing paragraphs instead of appending incident
histories. Exact layouts, thresholds, enum numbers, error lists, method signatures,
implementation sequences, and benchmark results belong with their code, tests,
or focused internal contract references. The references below preserve
cross-language rules needed to replace one implementation without reverse
engineering another; they are not public application-authoring guides.

| Question | Current authoritative source |
| --- | --- |
| Protobuf fields and fragment shapes | `crates/webui-protocol/proto/webui.proto` |
| Projection boundary, manifest identity, and hash contract | [`specs/projection.md`](specs/projection.md); `packages/webui/src/projection/{graph,manifest,diagnostics}.ts`, `crates/webui-protocol/src/projection_manifest.rs` |
| Parser directives and build diagnostics | `crates/webui-parser/src/`, `docs/guide/concepts/directives/` |
| State paths and expression semantics | `crates/webui-state/src/`, `crates/webui-expressions/src/`, `docs/guide/concepts/state-management/index.md`, `docs/guide/concepts/directives/if.md` |
| Rust render and streaming APIs | `crates/webui-handler/src/`, `crates/webui/src/streaming.rs` |
| Progressive response wire and host-step invariants | [`specs/streaming.md`](specs/streaming.md); `crates/webui-handler/src/streaming/`, `packages/webui-framework/src/streaming-protocol.ts` |
| Client template metadata, SSR markers, and hydration | [`specs/hydration.md`](specs/hydration.md); `crates/webui-handler/src/plugin/webui.rs`, `packages/webui-framework/src/element/markers.ts`, `docs/guide/concepts/hydration.md` |
| Browser routing | `packages/webui-router/`, `docs/guide/concepts/routing.md` |
| Desktop IPC trust, admission, and envelope contract | [`specs/desktop-ipc.md`](specs/desktop-ipc.md); `crates/webui-desktop/src/ipc/`, `packages/webui-desktop/src/envelope.ts` |
| Desktop app and window APIs | `docs/guide/concepts/desktop.md`, `crates/webui-desktop/src/` |
| Host APIs and examples | `docs/guide/integrations/`, `docs/guide/cli/`, package READMEs, `examples/` |

Tests at each boundary are the executable compatibility checks. Add focused
tests when behavior changes, including producer/consumer parity for binary
artifacts, SSR/client agreement for templates and routes, and host parity for
rendering and streaming. Measure performance-sensitive changes in release
mode, including payload size and retained client memory; do not turn
experiment notes into architectural requirements.
