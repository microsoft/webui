# @microsoft/webui

High-performance server-side rendering framework. Compiles HTML templates into a binary protocol at build time and renders them with native speed at runtime - no JavaScript runtime overhead.

> 📖 **Full documentation, tutorials, and playground at [microsoft.github.io/webui](https://microsoft.github.io/webui)**

## Installation

```bash
npm install @microsoft/webui
```

The package automatically installs the correct platform-specific native binary
for Windows, macOS, and Linux on x64 or ARM64. The Node API requires that
native addon and surfaces loading errors directly. It never falls back to a
subprocess. Use the `webui` CLI explicitly for filesystem builds.

## Quick start

```js
import { build, Protocol } from "@microsoft/webui";

// Build templates into a protocol
const result = build({ appDir: "./src" });

// Decode and index once, then render repeatedly
const protocol = new Protocol(result.protocol, { plugin: "webui" });
const html = protocol.render({ name: "World", items: ["a", "b"] });
console.log(html.toString("utf8"));
```

## API

### `build(options: BuildOptions): BuildResult`

Compiles an application directory of HTML templates into a binary protocol.

```js
const result = build({
  appDir: "./src",        // Path to the template directory
  entry: "index.html",   // Entry file (default: "index.html")
  css: "link",           // CSS strategy: "link", "style", or "module"
  dom: "light",          // Optional: global Light + authored Shadow islands
  cssBundle: true,       // Merge component stylesheets into shared chunks
  plugin: "webui",       // Parser plugin name
  components: [],        // Additional component sources
  componentAssetRoots: ["settings-dialog"], // Static .webui.js asset roots
  metafile: true, // Return an esbuild-compatible asset graph
  cssFileNameTemplate: "[name]-[hash].[ext]", // CSS/component asset filename template
  cssPublicBase: "https://cdn.example.com/assets", // Optional CDN/public href base
  theme: "./themes/brand.json", // Optional design-token theme validation
});

// result.protocol  - Buffer containing the compiled protocol
// result.cssFiles  - Array of [filename, content, ...] pairs
// result.componentAssetFiles - Array of [filename, ESM content, ...] pairs
// result.metafile - Optional esbuild-compatible graph JSON
// result.warnings  - Array of non-fatal build advisory diagnostics
// result.stats     - { durationMs, fragmentCount, componentCount, cssFileCount, protocolSizeBytes, tokenCount }
```

Unwrapped components default to generated open Shadow roots. Set `dom: "light"`
to render unwrapped components as global Light DOM. A sole bare `<template>` is
also an explicit Light wrapper and is unwrapped; a sole
`<template shadowrootmode="open">` remains Shadow in either mode. Light CSS uses
ordinary selectors; `:host`, `:host-context`, and `::slotted` fail with
`unsupported-light-css`.

Static component assets use a root/chunk graph. Entry-reachable dependencies
remain external, dependencies used by one root stay inline, and dependencies
with an identical multi-root consumer set are emitted once as dynamic chunks.
Asset-only records are omitted from `result.protocol`. Component assets cannot
be combined with `<route>`.

When `theme` is provided, every required CSS token must exist in the theme
after local and ancestor custom-property definitions are excluded. A `var()`
usage with a literal fallback (e.g. `var(--brand, #000)`) is exempt. Missing
required tokens fail the build with a structured `missing-theme-token` error.
Misspelled literal-fallback tokens are returned as non-fatal warnings.

### Optional state projection

The build-only `@microsoft/webui/projection.js` subpath exposes the
bundler-neutral projection compiler and the supported esbuild adapter. esbuild
and TypeScript are optional peer dependencies, so applications that do not use
projection do not install or load them:

```bash
npm install -D esbuild typescript
```

```js
import * as esbuild from "esbuild";
import { esbuildProjection } from "@microsoft/webui/projection.js";

await esbuild.build({
  entryPoints: ["src/index.ts"],
  outdir: "dist",
  bundle: true,
  splitting: true,
  format: "esm",
  plugins: [esbuildProjection()],
});

const result = build({
  appDir: "./src",
  plugin: "webui",
  projectionManifests: ["./dist/webui-projection.json"],
});
```

esbuild runs once and emits both browser chunks and
`webui-projection.json`; WebUI then embeds the exact initial/navigation
surfaces into `protocol.bin`. The adapter uses esbuild's resolved graph and
emitted output membership. Source inputs and outputs receive exact hashes;
opaque inputs rely on their emitted output proof. Code splitting, dynamic
imports, output hashes, and external bundles remain application-owned.

Other bundler adapters can use the exported `AdapterContext`,
`compileProjection()`, and conformance fixtures without importing esbuild. The
package currently ships and supports `esbuildProjection()` as its official
adapter. Custom adapters provide a normalized JavaScript/TypeScript source
graph and exact emitted output bytes. Non-source bundler inputs stay outside
the semantic graph, so projection does not decode or hash their input bodies.

With no manifest, WebUI performs no JavaScript analysis and preserves full
state. Once any manifest is supplied, coverage is strict: every scripted
component compiled into the protocol must have exactly one entry. Shared
controls built as external bundles should emit their own manifest fragment,
then all fragments should be passed through `projectionManifests`.

Manifest keys are exact JavaScript `@observable` and `@attr` property names.
During hydration, an existing SSR host attribute wins over projected `@attr`
state. Runtime hosts never load TypeScript, esbuild, or the manifest.

### `new Protocol(protocol: Buffer, options?: ProtocolOptions)`

Decodes and indexes a compiled protocol once. Keep this object for the server
lifetime and use it for all runtime operations.

```js
const protocol = new Protocol(protocolBytes, { plugin: "webui" });
```

`Protocol` owns its decoded native state. The package does not keep a hidden
`WeakMap`, copy the source `Buffer`, or expose render functions that accept
protocol bytes on every request.

### `protocol.render(state: object | string, options?: RenderOptions): Buffer`

Renders state into a UTF-8 Node.js `Buffer`, the canonical buffered result for
direct HTTP, file, or socket writes:

```js
response.end(protocol.render({ title: "Hello" }));
```

Call `.toString("utf8")` explicitly when JavaScript string operations are
required.

### `protocol.prepareState(state)` / `protocol.renderPrepared(state, options?)`

Use an immutable prepared state only when the same snapshot will be rendered
repeatedly, such as a cached page shell or fan-out response:

```js
const state = protocol.prepareState({ title: "Cached" });
response.end(protocol.renderPrepared(state));
```

Preparation performs object serialization and native JSON parsing once.
`renderPrepared()` reuses the parsed native tree and returns bytes identical to
`render()` for the same state and options. The opaque handle is process-local,
cannot be serialized, and does not observe later mutations to the source object.

A prepared state retains its native tree until the handle is garbage-collected.
This trades resident native memory for avoiding repeated stringify/parse work.
Prepare a new snapshot whenever request state changes, and continue to use
`render()` for ordinary per-request state.

### `protocol.renderStream(state, onChunk, options?): void`

Renders with streaming output. Internal handler writes are coalesced around a
16 KiB target before the callback crosses into JavaScript. The callback runs
synchronously and its return value is ignored. In particular,
`response.write()` returning `false` does not pause native rendering, so this
API does not provide transport backpressure. A thrown callback error aborts
rendering immediately and is rethrown to the caller.

```js
protocol.renderStream(state, (chunk) => {
  response.write(chunk);
});
```

### `protocol.streamResponse(options?): StreamingSession`

Opens a runtime-discovered progressive session. Unlike `renderStream`, each
session call returns bytes so your server keeps the socket and backpressure.

```js
const session = protocol.streamResponse({
  entry: 'index.html',
  requestPath: '/',
});

res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8' });
let step = session.start(initialState);
await write(res, step.bytes);

while (!step.done) {
  const boundary = step.boundary;
  if (boundary) {
    const state = await loadBoundaryState(
      boundary.owner,
      boundary.name,
      boundary.key,
    );
    step = session.resume(boundary.instanceId, state, 'final');
  } else {
    step = session.advance();
  }
  await write(res, step.bytes);
}
res.end();

async function write(res, chunk) {
  if (res.write(chunk)) return;
  // An aborted client never emits 'drain', and surfaces as 'close', not
  // 'error' — so waiting on 'drain' alone would hang forever.
  await new Promise((ok, fail) => {
    const done = (error) => {
      res.off('drain', onDrain);
      res.off('close', onClose);
      if (error) fail(error);
      else ok();
    };
    const onDrain = () => done();
    const onClose = () => done(new Error('client disconnected'));
    res.once('drain', onDrain);
    res.once('close', onClose);
  });
}
```

| Member | Returns | Description |
|--------|---------|-------------|
| `start(state)` | `StreamStep` | Bytes through the first occurrence or terminal |
| `resume(instanceId, state, mode?)` | `StreamStep` | Only the pending occurrence's bytes through its checkpoint (`'final'` \| `'updatable'`) |
| `advance()` | `StreamStep` | Following parent bytes through the next occurrence or terminal |
| `update(instanceId, patch)` | `Buffer` | Projected state for a committed updatable occurrence |

`StreamStep` contains `bytes`, `done`, and optional
`{ instanceId, declarationId, owner, name, key }`. A descriptor requires
`resume`; no descriptor with `done: false` requires `advance`; `done: true`
means complete. `resume` is boundary-only, while `advance` carries following
parent or tail bytes. No sibling boundary workaround is required. The completed
step already contains tail and terminal bytes. Updates are valid between
`resume` and `advance`, insert no markup, and never rerun hydration. Sessions
are single-driver and independent; hold one per request.

### `inspect(protocol: Buffer): string`

Returns a JSON representation of the protocol for debugging.

```js
const json = inspect(protocol);
console.log(JSON.parse(json));
```

### `protocol.renderPartial(state, entryId, requestPath, inventoryHex): string`

Produces a JSON partial response for client-side navigation, including state, template metadata, condition closures, and route chain.

### `protocol.renderComponentTemplates(componentTags, inventoryHex): string`

Renders templates and styles for on-demand component loading (used by `Router.ensureLoaded()`). Returns a JSON string with `componentStyles`, `templates`, `templateFunctions`, and `inventory`. Uses the same inventory bitfield as partial navigation to avoid sending duplicates.

```js
const json = protocol.renderComponentTemplates(["settings-dialog"], inventoryHex);
const { templates, templateFunctions, componentStyles, inventory } = JSON.parse(json);
```

## CLI

The package also includes the `webui` CLI binary:

```bash
# Build templates to disk
npx webui build ./src --out ./dist

# Start a dev server
npx webui serve ./src --state ./data/state.json --port 3000

# Inspect a compiled protocol
npx webui inspect ./dist/protocol.bin
```

## Platform support

| OS | Architecture | Package |
|---|---|---|
| Windows | x64 | `@microsoft/webui-win32-x64` |
| Windows | arm64 | `@microsoft/webui-win32-arm64` |
| macOS | arm64 | `@microsoft/webui-darwin-arm64` |
| macOS | x64 | `@microsoft/webui-darwin-x64` |
| Linux | x64 | `@microsoft/webui-linux-x64` |
| Linux | arm64 | `@microsoft/webui-linux-arm64` |

Platform-specific packages are installed automatically as optional dependencies.

## License

[MIT](https://github.com/microsoft/webui/blob/main/LICENSE)
