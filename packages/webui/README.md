# @microsoft/webui

High-performance server-side rendering framework. Compiles HTML templates into a binary protocol at build time and renders them with native speed at runtime - no JavaScript runtime overhead.

> 📖 **Full documentation, tutorials, and playground at [microsoft.github.io/webui](https://microsoft.github.io/webui)**

## Installation

```bash
npm install @microsoft/webui
```

The package automatically installs the correct platform-specific native binary
for your OS and architecture (Windows, macOS, Linux - x64 and arm64). The Node
API requires that native addon and surfaces loading errors directly. It never
falls back to a subprocess. Use the `webui` CLI explicitly for filesystem
builds.

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

The build-only projection subpaths keep the bundler-neutral compiler separate
from bundler adapters:

| Subpath | Purpose |
|---|---|
| `@microsoft/webui/projection/core.js` | `AdapterContext`, compiler, manifest API, and `ProjectionSession` |
| `@microsoft/webui/projection/esbuild.js` | Supported esbuild adapter |
| `@microsoft/webui/projection/rspack.js` | Supported Rspack adapter |
| `@microsoft/webui/projection/testing.js` | Shared conformance fixtures for adapter authors |
| `@microsoft/webui/projection.js` | Backward-compatible core + esbuild facade |

esbuild, Rspack, and TypeScript are optional peer dependencies, so applications
install only the peers used by their selected adapter:

```bash
npm install -D esbuild typescript
```

The compatible `@microsoft/webui/projection.js` facade does not export Rspack.
Importing the focused Rspack subpath keeps projects without `@rspack/core`
unaffected.

```js
import * as esbuild from "esbuild";
import { esbuildProjection } from "@microsoft/webui/projection/esbuild.js";

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
emitted output membership, so code splitting, dynamic imports, output hashes,
and external bundles remain application-owned.

Other bundler adapters can construct an exact `AdapterContext` and use the
shared finalization lifecycle without importing esbuild:

```js
import { ProjectionSession } from "@microsoft/webui/projection/core.js";

const session = new ProjectionSession();
await session.finalize(context);
```

`ProjectionSession` compiles, canonically serializes, and atomically replaces
the manifest. The adapter still owns graph extraction, path/root derivation,
bundler version checks, and native diagnostic presentation. Import
`runConformanceSuite` from `@microsoft/webui/projection/testing.js` to verify the
normalized compiler contract.

An esbuild plugin cannot be passed to Rspack because their plugin APIs differ,
so WebUI provides a native Rspack adapter:

```bash
npm install -D @rspack/core@^2.0.1 typescript
```

```typescript
import { fileURLToPath } from 'node:url';
import { rspackProjection } from '@microsoft/webui/projection/rspack.js';
import { rebuildProtocol } from './build-protocol.js';

export default {
  output: { path: fileURLToPath(new URL('./dist', import.meta.url)) },
  plugins: [
    rspackProjection({
      manifest: './dist/webui-projection.json',
      afterManifest: ({ manifestPath }) =>
        rebuildProtocol({ projectionManifest: manifestPath }),
    }),
  ],
};
```

The adapter reads Rspack 2.x's dependency graph, expands concatenated modules,
preserves code-split chunk membership, and hashes exact in-memory asset bytes
after emit. It canonicalizes package identity and virtual IDs, handles
externals, and removes duplicate dependency records without parsing output text.

`afterManifest` runs after atomic manifest replacement and is awaited by Rspack.
Use it to rebuild `protocol.bin` with the new manifest before starting a
dependent SSR bundle. If the callback rejects, the compilation fails. The
callback receives `{ manifest, context, manifestPath, compiler, compilation }`.

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

Opens a progressive streaming session for an entry that declares `<boundary>`
directives. Unlike `renderStream`, the session **returns** each chunk, so your
server keeps the socket, the write order, and the backpressure contract.

```js
import { once } from 'node:events';

const session = protocol.streamResponse({ entry: 'index.html', requestPath: '/' });
const status = session.boundary('job-status');   // resolve names once

res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8' });
await write(res, session.writeShell(baseState));
await write(res, session.writeBoundary(status, statusState, 'updatable'));

// Patches an already-hydrated island on this same response.
await write(res, session.update(status, { jobState: 'succeeded' }));
res.end(session.finish({}));

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
| `boundary(name)` | `number` | Integer handle for an authored boundary name |
| `boundaryCount` | `number` | Boundaries declared by the entry |
| `finished` | `boolean` | Whether `finish()` has been called |
| `writeShell(state)` | `Buffer` | Document prefix through the first semantic flush |
| `writeBoundary(id, state, mode?)` | `Buffer` | One boundary's markup and checkpoint (`'final'` \| `'updatable'`) |
| `update(id, state)` | `Buffer` | Projected state patch to an updatable boundary |
| `finish(state)` | `Buffer` | Tail checkpoint, terminal record, and document suffix |

Ordering is enforced. A rejected call throws and leaves the session usable, so
invalid state does not cost you the response. Sessions are independent — hold
one per in-flight request.

### `inspect(protocol: Buffer): string`

Returns a JSON representation of the protocol for debugging.

```js
const json = inspect(protocol);
console.log(JSON.parse(json));
```

### `protocol.renderPartial(state, entryId, requestPath, inventoryHex): string`

Produces a JSON partial response for client-side navigation, including state, template metadata, condition closures, and route chain.

### `protocol.renderComponentTemplates(componentTags, inventoryHex): string`

Renders templates and styles for on-demand component loading (used by `Router.ensureLoaded()`). Returns a JSON string with `templateStyles`, `templates`, `templateFunctions`, and `inventory`. Uses the same inventory bitfield as partial navigation to avoid sending duplicates.

```js
const json = protocol.renderComponentTemplates(["settings-dialog"], inventoryHex);
const { templates, templateFunctions, templateStyles, inventory } = JSON.parse(json);
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
