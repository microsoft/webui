# @microsoft/webui

High-performance server-side rendering framework. Compiles HTML templates into a binary protocol at build time and renders them with native speed at runtime - no JavaScript runtime overhead.

> 📖 **Full documentation, tutorials, and playground at [microsoft.github.io/webui](https://microsoft.github.io/webui)**

## Installation

```bash
npm install @microsoft/webui
```

The package automatically installs the correct platform-specific native binary for your OS and architecture (Windows, macOS, Linux - x64 and arm64).

## Quick start

```js
import { build, Protocol } from "@microsoft/webui";

// Build templates into a protocol
const result = build({ appDir: "./src" });

// Decode and index once, then render repeatedly
const protocol = new Protocol(result.protocol, { plugin: "webui" });
const html = protocol.render({ name: "World", items: ["a", "b"] });
console.log(html);
```

## API

### `build(options: BuildOptions): BuildResult`

Compiles an application directory of HTML templates into a binary protocol.

```js
const result = build({
  appDir: "./src",        // Path to the template directory
  entry: "index.html",   // Entry file (default: "index.html")
  css: "link",           // CSS strategy: "link" or "style"
  plugin: "webui",       // Parser plugin name
  components: [],        // Additional component sources
  componentAssetRoots: ["settings-dialog"], // Static .webui.js asset roots
  cssFileNameTemplate: "[name]-[hash].[ext]", // CSS/component asset filename template
  cssPublicBase: "https://cdn.example.com/assets", // Optional CDN/public href base
  theme: "./themes/brand.json", // Optional design-token theme validation
  outDir: "./dist",      // Output directory for CLI fallback
});

// result.protocol  - Buffer containing the compiled protocol
// result.cssFiles  - Array of [filename, content, ...] pairs
// result.componentAssetFiles - Array of [filename, ESM content, ...] pairs
// result.warnings  - Array of non-fatal build advisory diagnostics
// result.stats     - { durationMs, fragmentCount, componentCount, cssFileCount, protocolSizeBytes, tokenCount }
```

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
emitted output membership, so code splitting, dynamic imports, output hashes,
and external bundles remain application-owned.

Other bundler adapters can use the exported `AdapterContext`,
`compileProjection()`, and conformance fixtures without importing esbuild. The
package currently ships and supports `esbuildProjection()` as its official
adapter.

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

### `protocol.render(state: object | string, options?: RenderOptions): string`

Renders state and returns the full HTML string.

```js
const html = protocol.render({ title: "Hello", show: true });
```

### `protocol.renderBuffer(state: object | string, options?: RenderOptions): Buffer`

Renders state into a UTF-8 Node.js `Buffer`. This avoids creating a JavaScript
string and is the preferred buffered path when writing directly to an HTTP
response:

```js
response.end(protocol.renderBuffer({ title: "Hello" }));
```

### `protocol.renderStream(state, onChunk, options?): void`

Renders with streaming output. Internal handler writes are coalesced around a
16 KiB target before the callback crosses into JavaScript.

```js
protocol.renderStream(state, (chunk) => {
  response.write(chunk);
});
```

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
