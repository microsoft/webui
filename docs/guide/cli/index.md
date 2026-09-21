# CLI Reference

The `webui` command-line tool is the primary way to build WebUI applications. It takes your app folder containing HTML templates and web components, and produces the WebUI protocol output ready for server-side rendering.

## Installation

Install via npm:

```bash
npm install @microsoft/webui
```

Or install via Cargo for standalone CLI use:

```bash
cargo install microsoft-webui-cli
```

## Commands

WebUI Press is a separate native binary. Both `webui-press build` and
`webui-press serve` accept `--show=all|content` (default `all`) to generate
the complete site or only page content. See [WebUI Press](/guide/webui-press)
for configuration, content-mode behavior, and template regions.

### Global options

These flags work with any command:

| Option | Description | Default |
|--------|-------------|---------|
| `--format <FORMAT>` | Output format: `human` (colorized terminal) or `json` (machine-readable diagnostics on stdout) | `human` |

Use `--format json` in editors, CI, or AI/agent tooling that needs to parse build errors programmatically instead of scraping colorized terminal text. See [Error output and exit codes](#error-output-and-exit-codes).

### `webui build`

Build a WebUI application from an app folder.

```bash
webui build [APP] --out <OUT> [--entry <FILE>] [--css <MODE>] [--dom <MODE>] [--css-bundle] [--plugin <NAME>] [--components <SOURCE>]... [--projection-manifest <PATH>]... [--emit-component-assets <TAGS>] [--metafile <PATH>] [--theme <VALUE>] [--asset-file-name-template <TEMPLATE>] [--css-public-base <BASE>] [--legal-comments <MODE>]
```

**Arguments:**

| Argument | Description | Default |
|----------|-------------|---------|
| `APP` | Path to the app folder | `.` (current directory) |
| `--out <OUT>` | Output folder for protocol and assets, or a `.bin` file path to set the protocol filename (e.g. `./dist/app1.bin`) | *(required)* |
| `--entry <FILE>` | Entry HTML file name | `index.html` |
| `--css <STRATEGY>` | CSS delivery strategy: `link`, `style`, or `module` | `link` |
| `--dom <MODE>` | Fallback for components without an authored Shadow root: `shadow` or `light` | `shadow` |
| `--css-bundle` | Merge component stylesheets into shared chunks. Composes with `--css`; rejected with `--css module`. | *(off)* |
| `--plugin <NAME>` | Load a parser plugin | *(none)* |
| `--components <SOURCE>` | Additional component sources (npm packages or local paths). Repeatable. | *(none)* |
| `--projection-manifest <PATH>` | Bundler projection manifest fragment. Repeatable and valid only with `--plugin=webui`. | *(none; full state)* |
| `--emit-component-assets <TAGS>` | Comma-separated root component tags to emit as static WebUI component assets in `--out` | *(none)* |
| `--metafile <PATH>` | Write an esbuild-compatible component asset graph. Requires `--emit-component-assets`. | *(none)* |
| `--theme <VALUE>` | Design token theme to validate against: a JSON file path or npm package name. Missing required tokens fail the build. | *(none)* |
| `--asset-file-name-template <TEMPLATE>` | Emitted asset filename template for Link-mode CSS files and static component assets. Tokens: `[name]`, `[hash]`, `[ext]` | `[name].[ext]` |
| `--css-public-base <BASE>` | Optional public URL/path prefix for Link-mode CSS hrefs | *(none)* |
| `--legal-comments <MODE>` | Legal comment handling: `inline` preserves legal CSS comments, `none` strips all comments | `inline` |

Path inputs for `APP`, `--state`, `--servedir`, `--projection-manifest`, and
`--metafile` support absolute paths, relative paths, `~/...`, and `file://...`
URI-style values.

**CSS Modes:**

| Mode | Behavior |
|------|----------|
| `link` | Emits external `.css` files and installs their `<link>` resources in compiler-defined cascade order. |
| `style` | Installs compiled CSS in `<style>` elements. No separate CSS files are written. |
| `module` | Delivers compiled CSS with an SSR fallback and shares imported CSS module stylesheets across component instances when supported. No separate CSS files are written. |

All modes support Light and Shadow components. A component's ordinary paired
CSS file remains authored/global CSS in Light DOM and remains native Shadow CSS
in Shadow DOM. Resources are installed once per Document or ShadowRoot in
first-discovery order, including partial navigation, streaming, and static
component assets. Full-document SSR installs Document resources before
`</head>`. When the document omits an explicit head, resources precede document
content while remaining immediately after any leading doctype.
Document fragment renders install resources before fragment content; a Shadow
component rendered directly as the entry installs them inside
its declarative root.

For long-lived CDN/browser caching, include `[hash]` in
`--asset-file-name-template`. `[hash]` is the emitted file's SHA-256 content hash
truncated to 8 hex characters. Link-mode CSS files are still written to `--out`;
`--css-public-base` only changes the CSS href stored in `protocol.bin` and
emitted in `<link>` tags. Templates must be ASCII filenames. URL delimiters
(`#`, `%`, and `?`), path separators, whitespace, control characters, and
Windows-reserved filename characters are rejected.

**CSS bundling:**

Every component stylesheet is render-blocking, so one file per component costs a
request each and forfeits cross-file compression. `--css-bundle` merges component
stylesheets into shared chunks:

```bash
webui build ./my-app --out ./dist --css link --css-bundle
```

It composes with `--css` rather than replacing it: bundling decides how
stylesheets are *grouped*, `--css` decides how they *reach the page*. A Link
build gets fewer `<link>` tags and requests, and a Style build gets fewer inline
blocks.

Chunks split rather than duplicate. Only components reached by an identical set
of CSS trees share a chunk, so a stylesheet used by several routes lands in its
own chunk and is downloaded and cached once instead of being copied into every
route bundle. Cascade order is preserved exactly: a chunk's members must be
adjacent and identically ordered in every closure that contains them. The
compiler verifies both properties and splits any incompatible chunk.

Chunks are named `_chunk-<first-member>-<count>`, or the component's own tag when
a chunk has a single member. The leading underscore keeps multi-member resource
IDs distinct from legal component tags. Link builds retain per-component files
as independently loaded component and older-handler fallbacks, but current
handlers link only chunks on the bundled path, so the fallbacks add no requests.

Pair bundling with content-hashed filenames so chunks can be served immutably:

```bash
webui build ./my-app --out ./dist --css link --css-bundle \
  --asset-file-name-template "[name]-[hash].[ext]"
```

The default template is `[name].[ext]`, which emits `_chunk-nav-4.css`. That name
is stable across builds even when the CSS inside it changes, so it cannot carry a
long `Cache-Control: max-age=…, immutable`. With `[hash]` the same chunk becomes
`_chunk-nav-4-36c58ce5.css` and changes only when its bytes change, which is what
makes a shared chunk worth sharing: it stays in cache across deploys and across
routes.

Measured on a 26-component example over HTTP/2 with Brotli, bundling is a byte
and CSSOM optimization first: 14% fewer compressed CSS bytes (identical rules
compress better in fewer, larger files) and 27% fewer `CSSStyleSheet` objects,
both deterministic. Load-time metrics improve by low single-digit percentages.
The win is substantially larger over HTTP/1.1, where request count is bounded by
head-of-line blocking rather than multiplexed.

`--css-bundle` is rejected with `--css module`, which already inlines every
stylesheet as a data URI: there is no request to merge, and module specifiers are
resolved per component at compile time. Bundling is off by default, so protocol
size and emitted resource names are unchanged unless you opt in.

**Component assets:**

Use `--emit-component-assets` with the WebUI plugin to prebuild CDN-loadable
template assets for deferred UI such as dialogs loaded without
`@microsoft/webui-router`:

```bash
webui build ./my-app --out ./dist --plugin=webui \
  --emit-component-assets mail-thread,compose-page \
  --metafile ./dist/component-assets-meta.json
```

The flag is a strict comma-separated allowlist. Every tag must be a discovered
lowercase kebab-case component. Requested roots are compiled through synthetic
non-entry fragments, so they do not become part of initial SSR unless your entry
template also references them. A build containing both component assets and a
`<route>` fails with `component-assets-with-routes`; use the router's normal
partial-navigation pipeline for routed components.

Assets are ESM graph modules. Entry-reachable components stay in `protocol.bin`
and the application bundle, and become external prerequisites instead of being
copied. A dependency used by one asset root stays inline in that root.
Dependencies shared by the same two or more roots are emitted once as
`chunk-<first-sorted-component>.webui.js`, and each root dynamically imports the
chunks it needs. Requested-root order does not change ownership, bytes, or
hashes. Asset-only records are removed from `protocol.bin`.

Component assets use version 3 with a required, atomically registered
`componentStyles` catalog. Other versions and assets without the catalog are
rejected before registration.

`--metafile` writes esbuild-compatible `inputs` and `outputs`, including every
root-to-chunk `dynamic-import` edge and exact byte attribution. It can be opened
directly in an esbuild bundle analyzer or consumed by build tooling. The
metafile path is collision-checked with protocol, CSS, root, and chunk outputs
before any files are written.

FAST plugin builds can emit the same graph with `<f-template>`
payloads, but need a FAST-owned runtime loader. Every module intentionally omits
inventory state because a static CDN asset cannot know the page's loaded
template bitset. Use `--asset-file-name-template "[name]-[hash].[ext]"` for
long-lived CDN caching; `[hash]` is each module's SHA-256 content hash truncated
to 8 hex characters.

Load an asset before creating the component:

```typescript
import { mailAssets } from './lazy-assets.js';

mailAssets.preload('mail-thread');
panelSlot.replaceChildren(await mailAssets.create('mail-thread'));
```

```typescript
// lazy-assets.ts
import { defineComponentAssets } from '@microsoft/webui-framework/component-asset.js';

export const mailAssets = defineComponentAssets({
  'mail-thread': {
    asset: '/mail-thread.webui.js',
    module: () => import('./mail-thread/mail-thread.js'),
    data: async () => await (await fetch('/mail-thread-data.json')).json(),
  },
});
```

Keep the lazy component tag out of SSR-reachable templates unless it should be
eligible for initial SSR. Use a mount element or another non-HTML trigger, then
create the custom element with `mailAssets.create(...)`. The application must
load its normal entry bundle before component assets because entry-reachable
dependencies are external prerequisites. For Shadow builds, the compiler records final Link stylesheet hrefs in the
protocol so `preload(tag)` can start CSS beside the authored stable root asset
without exposing content-hashed stylesheet names. Light builds emit those hrefs
as document stylesheets with the entry because their CSS is globally scoped.

**Comment handling:**

WebUI strips HTML comments and CSS comments at build time. Bindings or
directives inside HTML comments are ignored and never produce fragments or
hydration metadata. Inside `<style>` tags, dynamic CSS fragments are valid only
when wrapped as exact CSS block comments, such as `/*{{{tokens.light}}}*/`.
With the default `--legal-comments inline`, CSS comments that contain
`@license` or `@preserve`, or start with `/*!` or `//!`, are preserved inline.
Use `--legal-comments none` to strip all non-signal comments.

**Component DOM ownership:**

Shadow is the backward-compatible default: unwrapped component content receives
a compiler-generated open Shadow root. Pass `--dom light` to render unwrapped
components as direct Light DOM children with authored/global CSS in their
owning CSS tree. Light CSS is not selector-rewritten or marker-scoped, so
ordinary selectors can reach other Light DOM in that tree. In either build mode,
a sole bare top-level `<template>` explicitly selects Light and is unwrapped.
A sole top-level `<template shadowrootmode="open">` is authoritative and keeps
that component Shadow, so either build can contain explicit Shadow islands.
Templates with attributes and policy wrappers such as `w-render` remain
ordinary/policy content rather than selecting a mode.

`:host`, `:host(...)`, `:host-context(...)`, and `::slotted(...)` are Shadow-only
and fail with `unsupported-light-css` in effective Light CSS. Use ordinary
selectors such as the component tag, or author an open Shadow root.

Closed roots and invalid values or placement always fail the build. Native
`<slot>` is allowed in effective Shadow components and rejected in effective
Light components.

FAST 2/3 plugins currently require effective Shadow components. Combining
`--plugin fast`, `fast-v2`, or `fast-v3` with an effective Light component
fails with `fast-light-dom-unsupported` instead of allowing the FAST client
runtime to replace Light SSR with a Shadow root.

See [Performance - Light DOM vs Shadow DOM](/guide/concepts/performance#light-dom-vs-shadow-dom) for benchmarks and guidance.

**Examples:**

```bash
# Build from current directory
webui build --out ./dist

# Build a specific app folder
webui build ./my-app --out ./dist

# Use a custom entry file
webui build ./my-app --out ./dist --entry home.html

# Opt into mixed Light DOM with authored Shadow islands
webui build ./my-app --out ./dist --dom light

# Build with style CSS (no external CSS files)
webui build ./my-app --out ./dist --css style

# Build link-mode CSS with content-hashed filenames
webui build ./my-app --out ./dist --asset-file-name-template "[name]-[hash].[ext]"

# Point generated stylesheet hrefs at a CDN/public asset root
webui build ./my-app --out ./dist \
  --asset-file-name-template "[name]-[hash].[ext]" \
  --css-public-base "https://cdn.example.com/assets"

# Build with the WebUI Framework plugin (hydration support)
webui build ./my-app --out ./dist --plugin=webui

# Build browser code first, then embed exact state projection metadata
node ./my-app/build-client.mjs
webui build ./my-app --out ./dist --plugin=webui \
  --projection-manifest ./my-app/dist/webui-projection.json

# Build with external component packages
webui build ./my-app --out ./dist --components @reactive-ui

# Validate CSS design tokens against a theme
webui build ./my-app --out ./dist --theme ./themes/brand.json

# Build with components from a local shared library
webui build ./my-app --out ./dist --components ./shared/components

# Customize the protocol filename (useful when building multiple apps to one folder)
webui build ./src/apps/app1 --out ./dist/app1.bin
webui build ./src/apps/app2 --out ./dist/app2.bin
```

`--projection-manifest` is opt-in and strict. Without it, WebUI performs no
JavaScript analysis and preserves full state. With one or more fragments, every
scripted component compiled from the app or `--components` sources must have
exactly one manifest entry. Build external component bundles separately and
repeat the flag for each fragment. See
[Build-Time State Projection](/guide/concepts/hydration#build-time-state-projection).

For progressive pages, use the
[bundler-independent coordinator delivery contract](/guide/concepts/hydration#separate-coordinator-and-application-assets)
to keep application startup separate. The host's existing asset handoff owns
script URLs; there is no additional streaming manifest input. Authored module
scripts with `fetchpriority="low"` are excluded from automatic modulepreload
hints so deferred application code does not get promoted into the head.

### `webui inspect`

Inspect a `protocol.bin` file by converting it to JSON and printing to stdout. Useful for debugging and piping to tools like `jq`.

```bash
webui inspect <FILE>
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `FILE` | Path to a `protocol.bin` file |

**Examples:**

```bash
# Inspect a protocol file
webui inspect dist/protocol.bin

# Pretty-print a specific fragment with jq
webui inspect dist/protocol.bin | jq '.fragments["index.html"]'

# Count total fragments
webui inspect dist/protocol.bin | jq '.fragments | keys | length'
```

### `webui serve`

Start a development server that builds, renders, and serves a WebUI application. Enable live reload with `--watch`.

```bash
webui serve [APP] --state <FILE> [--servedir <DIR>] [--watch] [--port <PORT>] [--entry <FILE>] [--css <MODE>] [--dom <MODE>] [--css-bundle] [--plugin <NAME>] [--components <SOURCE>]... [--projection-manifest <PATH>]... [--api-port <PORT>] [--emit-component-assets <TAGS>] [--metafile <PATH>] [--theme <VALUE>] [--asset-file-name-template <TEMPLATE>] [--css-public-base <BASE>] [--legal-comments <MODE>]
```

**Arguments:**

| Argument | Description | Default |
|----------|-------------|---------|
| `APP` | Path to the template/component directory | `.` (current directory) |
| `--state <FILE>` | Path to JSON state file for rendering | *(required)* |
| `--servedir <DIR>` | Directory served at `/*` | *(optional)* |
| `--watch` | Enable file watching + HMR | `false` |
| `--shutdown-timeout <SECONDS>` | Opt in to supervised shutdown with a positive integer grace period, with or without `--watch` | *(none)* |
| `--port <PORT>` | Port to bind the development server | `3000` |
| `--entry <FILE>` | Entry HTML file name | `index.html` |
| `--css <MODE>` | CSS delivery strategy: `link`, `style`, or `module` | `link` |
| `--dom <MODE>` | Fallback for components without an authored Shadow root: `shadow` or `light` | `shadow` |
| `--css-bundle` | Merge component stylesheets into shared chunks. Composes with `--css`; rejected with `--css module`. | *(off)* |
| `--plugin <NAME>` | Load parser + handler plugins (e.g., `webui`) | *(none)* |
| `--components <SOURCE>` | Additional component sources (npm packages or local paths). Repeatable. | *(none)* |
| `--projection-manifest <PATH>` | Bundler projection manifest fragment. Repeatable and valid only with `--plugin=webui`. | *(none; full state)* |
| `--api-port <PORT>` | Proxy route requests to your API server. JSON responses provide buffered state; `application/x-webui-stream` responses drive progressive boundary rendering. Encoded paths and queries are forwarded unchanged. | *(none)* |
| `--emit-component-assets <TAGS>` | Comma-separated root component tags to compile as static WebUI component assets, matching `webui build`. Their templates and CSS are parsed and validated on every build, and the compiled `<tag>.webui.js` modules are served from memory. | *(none)* |
| `--metafile <PATH>` | Atomically replace an esbuild-compatible component asset graph after each successful build. Requires `--emit-component-assets`. | *(none)* |
| `--theme <VALUE>` | Design token theme: a path to a JSON file or an npm package name. Missing required tokens fail the build; resolved tokens are injected into the render state. | *(none)* |
| `--asset-file-name-template <TEMPLATE>` | Emitted asset filename template for Link-mode CSS files. Tokens: `[name]`, `[hash]`, `[ext]` | `[name].[ext]` |
| `--css-public-base <BASE>` | Optional public URL/path prefix for Link-mode CSS hrefs | *(none)* |
| `--legal-comments <MODE>` | Legal comment handling: `inline` preserves legal CSS comments, `none` strips all comments | `inline` |

The `APP` directory should contain your entry HTML and component files.

#### Bounded dev-server shutdown

Both `webui serve` and `webui-press serve` accept `--shutdown-timeout`:

```bash
webui serve ./src --watch --shutdown-timeout 10
webui-press serve --shutdown-timeout 10
```

Without the flag, shutdown waits for the active rebuild to finish with no
deadline. With it, one supervised server process remains alive across rebuilds,
retaining the warm build cache. A first Ctrl-C (or Unix SIGTERM or SIGHUP when
supported by the platform signal handler) requests HTTP stop, then waits for the
active rebuild within the specified grace period. Existing HTTP connections are
stopped rather than drained.
A second stop request or an expired grace period terminates the owned server
process tree and returns a nonzero status. The flag also applies during initial
build and when `webui serve` runs without `--watch`.

The supervisor allows up to two additional seconds to confirm the server child
exited. Forced termination can leave incomplete generated files; rebuild before
using them. Normal child failures retain their exit codes.

Containment uses Windows Job Objects or Unix process groups. It does not cover
descendants that escape containment or daemonize, uninterruptible kernel tasks,
or force-killing the supervisor. OS scheduling means the timeout is not a hard
real-time guarantee. In supervised mode, stdin is reserved for shutdown control.
On Unix, output is relayed by the foreground supervisor so terminals with
`TOSTOP` do not suspend the contained server when it writes output.

**What it does:**

1. Builds the protocol from your `APP` directory (no separate `webui build` step needed)
2. Renders the entry template with state data
3. Serves the rendered HTML with an injected live-reload script
4. If `--watch` is enabled, watches app, state, asset, and explicit projection manifest files for changes
5. If `--watch` is enabled, automatically rebuilds and re-renders when files change
6. If `--watch` is enabled, connected browsers reload automatically via the polling HMR backend

When `--api-port` is set, backend state requests and `/api/*` forwarding use
the encoded path and query exactly as received except for the entry route alias.
`/` and `/index.html` both resolve backend state at `/` (the entry path is
normalized), while still preserving the query string. All other request paths
forward their encoded path and query unchanged. Do not double-encode route
parameters for development. For example, `%2F` remains part of one parameter
instead of becoming a path separator.

For progressive HTML, the server sends
`Accept: application/x-webui-stream, application/json` to the API backend. A
backend can return a versioned NDJSON control stream:

```text
{"type":"start","version":2,"state":{"query":""}}
{"type":"resume","boundary":{"owner":"ntp-page","name":"search-ready"},"state":{"query":""},"mode":"updatable"}
{"type":"update","boundary":{"owner":"ntp-page","name":"search-ready"},"state":{"query":"webui"}}
```

`start` appears once. It renders until the first runtime occurrence or terminal.
Each `resume.boundary` must match the descriptor currently returned by WebUI
using `owner`, `name`, and `key`; omit `key` only when that descriptor has none.
An optional `declarationId` can tighten the match. Resume `state` is passed to
that occurrence and `mode` is `final` by default or `updatable`.
`update.boundary` uses the same identity to target one previously committed
updatable occurrence and requires object-valued `state`.

The control stream has no `advance` record because the CLI drives that core
operation:

| Core step state | CLI action |
|---|---|
| descriptor present | Wait for the matching `resume` control and call core `resume` |
| no descriptor and not done | Call core `advance` |
| done | Complete the browser response |

Core `resume` emits only the pending occurrence through its checkpoint. Core
`advance` emits the following parent or tail bytes through the next descriptor
or terminal. After the backend sends the resume for the final descriptor and
closes its NDJSON body, the CLI's final `advance` emits the terminal. There is
no separate end command.

The CLI owns response-local instance IDs and the browser transport. A
capacity-one command channel preserves backpressure, and each record is capped
at 2,000,000 bytes. Before HTTP 200, bytes from `start` are staged without
copying up to a 4,000,000-byte limit. Dropping the browser response cancels the
backend stream. The backend must honor its HTTP writer's backpressure signal and
cap concurrent streams. Returning JSON retains ordinary buffered behavior. See
[`<boundary>`](/guide/concepts/directives/boundary) and
`examples/app/streaming`.

If the backend is unreachable, returns state the server cannot parse, or answers
a stream request with a non-success status such as `503` from its concurrency
cap, `webui serve` logs one warning and still renders the page from fallback
state. A refused request never started a stream, so it degrades the same way an
unreachable backend does instead of replacing your app with the upstream error
body. A failure that occurs *after* the stream is live still fails the response,
because bytes already sent to the browser cannot be rewound.

After generated assets and `--servedir` files miss, route fallback is based on
the `Accept` header. Requests that explicitly accept `text/html` or
`application/xhtml+xml` receive the SSR document, and requests that explicitly
accept `application/json` receive the JSON partial response. `q=0` disables
that media type, while a malformed or out-of-range `q` value falls back to
`q=1.0`; when HTML and JSON are both acceptable, the higher `q` wins and exact
ties prefer JSON. Missing or wildcard-only `Accept` headers return 404, as do JS,
CSS, image, and other
non-HTML/non-JSON asset requests. Dots are valid in route segments, so paths
such as `/docs/v2.1` can still fall back to the route renderer.

**Examples:**

```bash
# Start serving the current directory
webui serve . --state ./state.json --servedir ./assets

# Start serving a specific templates directory
webui serve ./examples/app/hello-world/templates --state ./examples/app/hello-world/data/state.json --servedir ./examples/app/hello-world/assets --watch

# Use a custom port
webui serve ./my-app --state ./state.json --servedir ./assets --port 9090 --watch

# Use style CSS mode
webui serve ./my-app --state ./state.json --servedir ./assets --css style --watch

# Use the WebUI Framework plugin for hydration
webui serve ./my-app --state ./state.json --plugin=webui --port 3001

# Rebuild when the client bundler atomically replaces its manifest
webui serve ./my-app --state ./state.json --plugin=webui \
  --projection-manifest ./dist/webui-projection.json --watch

# Dev server with external components (--watch watches local paths)
webui serve ./my-app --state ./state.json --components @reactive-ui --watch

# Proxy route requests to your API server (e.g. Express on port 4000)
webui serve ./my-app --state ./state.json --api-port 4000 --watch

# Apply a design token theme from an npm package
webui serve ./my-app --state ./state.json --theme @my-org/brand-tokens --watch

# Apply a design token theme from a local JSON file
webui serve ./my-app --state ./state.json --theme ./themes/dark.json --watch
```

When `--theme` is present on `build` or `serve`, every required token must
exist in every theme. Nested fallback tokens are validated individually:
`var(--a, var(--b, var(--c)))` requires `a`, `b`, and `c` unless a token is
defined by local or ancestor CSS. A `var()` usage with a literal fallback (e.g.
`var(--brand, #000)`) is exempt — the token is still hoisted for runtime
resolution but its absence does not fail the build. When such a literal-fallback
token is also absent from every theme it is surfaced as a non-fatal
`unthemed-token` **warning** (rendered like an error, with location, snippet, and
a `did you mean …?` suggestion) since it is usually a typo.

`--emit-component-assets` behaves identically on `serve` and `build`: each listed
root is parsed and validated on every build - its template and CSS are checked
for HTML and theme-token errors even though the component is not part of the
initial SSR tree - so authoring mistakes in lazily loaded components fail the
dev build instead of being silently skipped. Root and shared chunk modules are
served from memory (and rebuilt on change under `--watch`), so no separate
`webui build` step or `--out` directory is needed during development. With
`--metafile`, a successful rebuild atomically replaces the graph; a failed
rebuild leaves the last valid metafile untouched. The metafile itself is ignored
by the watcher to prevent rebuild loops.

In `serve --watch`, rebuild failures are sticky: the terminal and live-reload
SSE report the error, and refreshing the page returns the latest rebuild error
instead of stale HTML while keeping the live-reload connection active. The next
successful rebuild clears the error and reloads connected browsers.

**Routes:**

| Path | Description |
|------|-------------|
| `/` or `/index.html` | Rendered HTML with live-reload script |
| `/*.webui.js` | In-memory root and shared component assets emitted by `--emit-component-assets` |
| `/*` | Static files from `--servedir` (when provided) |
| `/*` with `Accept: text/html`, `application/xhtml+xml`, or `application/json` at q > 0 after asset misses | SPA route fallback (highest q wins; JSON wins exact ties) |
| Missing JS, CSS, image, and wildcard-only asset requests | 404 |
| `/hmr` | HMR version endpoint (polling backend, only when `--watch`) |

### `webui desktop`

Run desktop tooling through `webui`, the only public CLI. Desktop support is
implemented by a separate `webui-desktop` sidecar backend so normal
build/serve/inspect installs stay lean and do not link native webview
dependencies. The sidecar is resolved automatically from the installed desktop
support package, next to the `webui` binary, or from the workspace during local
development; set `WEBUI_DESKTOP_BINARY` only to override discovery.

```bash
webui desktop init [APP_ROOT] [--force]
webui desktop run [APP] [--state <FILE>] [--servedir <DIR>] [--theme <VALUE>]
webui desktop build [APP] --out <BUNDLE_DIR> [--state <FILE>] [--servedir <DIR>] [--theme <VALUE>] [--entry <FILE>] [--css <MODE>] [--dom <MODE>] [--plugin <NAME>] [--components <SOURCE>]...
webui desktop package <APP_ROOT|BUNDLE_DIR> [--target <TARGET>] --out <OUT_DIR> [--theme <VALUE>] [--icon <FILE>] [--runner <PATH>] [--runner-crate <NAME>] [--debug] [--runner-features <FEATURES>] [--runner-default-features] [--bundle-out <DIR>] [--no-web-build]
```

`webui desktop init` creates a minimal `src/index.html`, `package.json`, and
`desktop/` Rust runner. It refuses to replace existing generated files; pass
`--force` when regenerating a scaffold.
The runner is a standalone Cargo workspace with an optimized release profile.
It depends on one desktop SDK and enables source compilation only when run with
`--features source`.

**Arguments:**

| Argument | Description | Default |
|----------|-------------|---------|
| `APP` | Path to the app folder | `.` |
| `--out <BUNDLE_DIR>` | Output desktop bundle directory | *(required)* |
| `--state <FILE>` | Startup state JSON copied into the bundle | *(optional)* |
| `--servedir <DIR>` | Static assets copied into the bundle | *(optional)* |
| `--theme <VALUE>` | Design token theme, as a file path or npm package name | *(optional)* |
| `--app-id <ID>` | Reverse-DNS app identifier | `com.microsoft.webui.app` |
| `--app-name <NAME>` | Human-readable app name | `WebUI App` |
| `--app-version <VERSION>` | App version stored in the bundle manifest | `0.0.0` |
| `--publisher <NAME>` | Publisher stored in the bundle manifest | `Microsoft` |
| `--title <TITLE>` | Default desktop window title | `WebUI` |
| `--width <PX>` | Default desktop window width | `1200` |
| `--height <PX>` | Default desktop window height | `800` |
| `--devtools` | Enable web inspector/devtools for the packaged desktop webview | `false` |
| `--theme <VALUE>` | Theme override for app-root packaging | `webuiDesktop.theme` |
| `--icon <FILE>` | App icon override for app-root packaging | `webuiDesktop.icon` |
| `--runner <PATH>` | App-specific runner executable for existing bundle packaging | sidecar runner |
| `--runner-crate <NAME>` | Cargo package name for app-root packaging | inferred from `desktop/Cargo.toml` |
| `--release` | Explicitly select the default optimized runner build | optimized by default |
| `--debug` | Build a debug runner instead of an optimized release runner | `false` |
| `--runner-features <FEATURES>` | Additional comma-separated Cargo features for the app-specific runner | `webuiDesktop.runnerFeatures` |
| `--runner-default-features` | Include the runner's default Cargo features | `webuiDesktop.runnerDefaultFeatures`, otherwise `false` |
| `--bundle-out <DIR>` | Keep the intermediate desktop bundle at this path | temporary bundle |
| `--no-web-build` | Skip configured `webuiDesktop.buildScripts` | `false` |

The bundle contains `protocol.bin`, generated CSS, copied static assets under
`assets/`, optional `state.json`, `manifest.webui-desktop.json`, and SHA-256
integrity hashes. Native window backends use system webviews only: WebView2 on
Windows, WKWebView on macOS, and GTK4/WebKitGTK 6 on Linux. Electron, Node,
bundled Chromium, and localhost HTTP servers are not part of desktop mode.

```bash
webui desktop build ./src \
  --state ./data/state.json \
  --servedir ./dist \
  --out ./desktop-bundle \
  --plugin webui \
  --theme @my-org/brand-tokens \
  --devtools \
  --app-id com.example.todo \
  --app-name "Todo Desktop"
```

On macOS, inspect the app from Safari's Develop menu. Enable it with Safari >
Settings > Advanced > Show features for web developers.

Package a Rust-first desktop app root in one command:

```bash
webui desktop package ./my-app --target macos-app --out ./packages
```

For app roots, `webui desktop package` reads `webuiDesktop` from `package.json`,
runs configured web build scripts, builds the app-specific Cargo runner crate
with `--release --no-default-features`,
stages non-generated assets, builds the bundle, and packages the runner-backed
app. Pass `--theme` to override `webuiDesktop.theme` for a one-off package.
Use `--debug` for a debug package. To include optional application capabilities,
set `webuiDesktop.runnerFeatures` to an array of Cargo feature names, or add
`--runner-features tray,native-dialogs` if your runner declares those features.
Use `runnerDefaultFeatures: true` or `--runner-default-features` only when the
runner intentionally needs its default features in production. These build
options do not change a prebuilt executable supplied with `--runner`.
Pass `--icon` to override `webuiDesktop.icon`; macOS packages use `.icns` icons
as `CFBundleIconFile`, and portable layouts copy the icon into resources.
Existing bundle packaging remains available:

```bash
webui desktop package ./desktop-bundle --target macos-app --out ./packages \
  --runner ./target/release/my-desktop-host
```

The current Rust packager writes runnable macOS `.app` bundles and portable
folder layouts. Omitting `--runner` for an existing bundle packages the generic
sidecar and is appropriate only for file-backed/static seed-state bundles.
Installer targets (`windows-msi`, `windows-msix`, `linux-appimage`, `linux-deb`,
`linux-rpm`) return actionable tooling diagnostics until their platform packagers
are enabled.

For an app-specific Rust runner, enable the native SDK and keep source
compilation opt-in:

```toml
[features]
default = []
source = ["microsoft-webui-desktop/source"]

[dependencies]
microsoft-webui-desktop = { version = "0.0.29", features = ["native"] }
```

`webui desktop package` builds this lean configuration automatically. For a
manual runner build:

```bash
cargo build --release -p my-desktop-runner --no-default-features
```

The lean runner still supports `DesktopRuntime::from_bundle`,
`DesktopRuntime::from_bundle_config`, and
`DesktopRuntime::from_bundle_config_and_manifest`. It does not expose source,
bundle-building, or package-building APIs. Run unpackaged development builds
with `cargo run --features source`. Build options are reexported by
`webui_desktop` when `source` is enabled; applications need no separate compiler
or runner dependency. The SDK's `cli` feature is for desktop tooling, not shipped
app code. See the [desktop SDK guide](../integrations/desktop.md) for the shared
source/bundle app builder and customization APIs.

## Error output and exit codes

When a template has an authoring mistake, the CLI prints a structured diagnostic with a stable error code, the source location, the offending snippet, and an actionable `help:` line:

```
✘ error: invalid <for> each expression [invalid-for-each]
  --> index.html:67:5
    each="person inpeople"
  help: use the form each="item in collection", e.g. each="todo in todos"
```

Where the mistake is likely a typo, the `help:` line suggests the intended name — a misspelled directive attribute (`eahc` → `each`) or an unregistered custom-element tag that closely matches a registered component **in the same namespace** (`<mp-buton>` → `<mp-button>`). A custom element in a different namespace (e.g. a third-party `<md-button>`) is left untouched and passes through to the browser.

### JSON diagnostics

With `--format json`, each error is emitted as a single JSON object on **stdout** (the colorized terminal output is suppressed), so editors, CI, and AI assistants can consume it directly:

```bash
webui build ./my-app --out ./dist --format json
```

```json
{
  "severity": "error",
  "code": "invalid-for-each",
  "message": "invalid <for> each expression",
  "file": "index.html",
  "line": 67,
  "column": 5,
  "snippet": "each=\"person inpeople\"",
  "help": "use the form each=\"item in collection\", e.g. each=\"todo in todos\"",
  "chain": ["Build failed", "Failed to parse index.html", "..."]
}
```

Fields that don't apply to a given error are `null`. The `code` is stable across releases — branch on it rather than on the human-readable `message`.

### Exit codes

The process exit code follows the BSD `sysexits.h` conventions so scripts and CI can branch on the cause:

| Code | Meaning |
|------|---------|
| `0` | Success |
| `1` | Generic failure |
| `2` | Invalid arguments / usage |
| `65` | Template or authoring error (`EX_DATAERR`) |
| `66` | Missing input: app folder, `--state` file, `--servedir`, or entry file (`EX_NOINPUT`) |
| `69` | Requested `--port` is already in use (`EX_UNAVAILABLE`) |
| `74` | I/O error reading or writing files (`EX_IOERR`) |

## App Folder Structure

The CLI expects your app folder to contain an entry HTML file and optionally web component files:

```
my-app/
├── index.html          # Entry template (or specify with --entry)
├── my-card.html        # Web component: <my-card>
├── my-card.css         # Component styles (auto-discovered)
├── nav-bar.html        # Web component: <nav-bar>
├── nav-bar.css         # Component styles
├── styles.css          # Global styles
└── app.js              # Client-side scripts
```

### Component Discovery

The CLI automatically discovers web components in your app folder:

- **HTML files with a hyphen** in the name are treated as components (e.g., `my-card.html` → `<my-card>`)
- **CSS files** with the same name are automatically paired (e.g., `my-card.css`)
- Components are registered and available for use in your templates
- Discovery is recursive - components in subdirectories are also found

### Entry Template

Your entry HTML file is a standard HTML document using WebUI directives:

```html
<!DOCTYPE html>
<html lang="en">
<head>
    <title>My App</title>
    <link rel="stylesheet" href="styles.css">
</head>
<body>
    <h1>Hello, {{name}}!</h1>

    <for each="item in items">
        <my-card>{{item.title}}</my-card>
    </for>

    <if condition="showFooter">
        <footer>Thanks for visiting</footer>
    </if>
</body>
</html>
```

## Build Output

The `--out` folder will contain:

```
dist/
├── protocol.bin        # The WebUI protocol (protobuf binary)
├── my-card.css         # Component CSS (--css link only)
└── nav-bar.css         # Component CSS (--css link only)
```

With `--css style`, only `protocol.bin` is written - CSS is embedded directly in the protocol's template fragments.

### protocol.bin

The protocol file contains a serialized `WebUIProtocol` structure (protobuf binary) with all parsed fragments. This file is consumed by a [platform handler](/guide/integrations/) at runtime to render HTML with your application state.

The binary format is not human-readable. The equivalent proto schema structure looks like:

```protobuf
// WebUIProtocol
fragments {
  key: "index.html"
  value: FragmentList {
    fragments: [
      Raw { value: "<h1>Hello, " },
      Signal { value: "name", raw: false },
      Raw { value: "!</h1>" },
      For { item: "item", collection: "items", fragment_id: "for-1" }
    ]
  }
  key: "for-1"
  value: FragmentList {
    fragments: [
      Component { fragment_id: "my-card" },
      Signal { value: "item.title", raw: false }
    ]
  }
}
```

## Error Messages

The CLI provides helpful error messages with suggestions:

```
  ✘ Failed to read /path/to/app/index.html
  caused by: No such file or directory (os error 2)

  hint: Try using --entry <file> to specify a different entry file
```

```
  ✘ App folder not found: /nonexistent/path
  caused by: No such file or directory (os error 2)

  hint: Check that the app folder path exists
```

## Plugins

The `--plugin` flag loads framework-specific extensions that customize both parsing and rendering behavior. The available plugin identifiers are listed in the [Plugins](/guide/concepts/plugins/) reference. No plugin is enabled by default — output is plain SSR HTML unless one is selected.

```bash
# Load a plugin by name
webui build ./my-app --out ./dist --plugin=<name>
webui serve ./my-app --state ./state.json --plugin=<name>
```

See [Plugins](/guide/concepts/plugins/) for detailed documentation.

## External Component Sources

The `--components` flag lets you discover components from npm packages or local directories outside your app folder. This is useful for shared component libraries.

### npm Packages

Pass an npm package name. The package must already be installed in `node_modules/`.
Use an unscoped name, `@scope`, or `@scope/package`, optionally followed by `/*`.
Package subpaths, traversal, and backslashes are not valid package identifiers.
For a filesystem directory, pass an explicit local path such as `./shared/components`.

```bash
# Single package
webui build ./my-app --out ./dist --components my-widget

# Scoped package (discovers all sub-packages)
webui build ./my-app --out ./dist --components @reactive-ui

# Specific scoped sub-package
webui build ./my-app --out ./dist --components @reactive-ui/button
```

**Default WebUI package requirements:**

Provide `<component-name>.html` files beneath the package's `components/`
directory, or the package root when no `components/` directory exists.
The filename determines the component name, including in nested directories.
Matching `.css` supplies styles; a matching `.ts` or `.js` sibling marks that
component as authored. Package exports and CEM metadata do not select or rename
native templates, and no manifest is required.

See
[External components](/guide/concepts/components#external-component-sources)
for the native package layout.
**Resolution:** The CLI searches ancestor `node_modules/` directories for the
requested package or scope, not merely the nearest `node_modules/`. Symlinks
(pnpm, npm workspaces) are resolved automatically. A bare scope searches its
nearest matching directory, skips unrelated packages, and reports failures
in declared component packages.
Collection spellings `@scope/*` and `@scope/package/*` select the same sources as
`@scope` and `@scope/package`; quote them to avoid shell glob expansion.

### Local Paths

Pass a filesystem path to discover components the same way the app directory
is scanned. A sibling `.ts` or `.js` file marks a component as
authored/interactive. Otherwise the component remains HTML-only.

```bash
# Relative path
webui build ./my-app --out ./dist --components ./shared/components

# Absolute path
webui build ./my-app --out ./dist --components /libs/ui-kit
```

### Multiple Sources

Combine multiple `--components` flags:

```bash
webui build ./my-app --out ./dist \
  --components @reactive-ui \
  --components ./shared/components \
  --components my-widget
```

### Caching

Discovered npm package components are cached at `~/.webui/cache/components/`.
Changes to the selected plugin's templates, stylesheets, scripts, or manifests
invalidate its cached result, including optional file creation and removal.
Default/WebUI/none discovery does not use package metadata, so metadata-only
`package.json` edits do not invalidate its cache. Plugins that use package
metadata, such as FAST, also invalidate on `package.json` changes.
Local path sources are always re-scanned.

## Next Steps

- [Hello World Tutorial](/tutorials/hello-world) - Build your first WebUI app
- [Components](/guide/concepts/components/) - Learn about web components
- [Template Directives](/guide/concepts/directives/) - `<for>`, `<if>`, and `{{}}`
- [Platform Handlers](/guide/integrations/) - Render protocols with state at runtime
