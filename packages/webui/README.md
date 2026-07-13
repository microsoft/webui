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
import { build, render } from "@microsoft/webui";

// Build templates into a protocol
const result = build({ appDir: "./src" });

// Render with state data
const html = render(result.protocol, { name: "World", items: ["a", "b"] });
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

### `render(protocol: Buffer, state: object | string): string`

Renders a compiled protocol with state data and returns the full HTML string.

```js
const html = render(protocol, { title: "Hello", show: true });
```

Keep the same protocol `Buffer` and plugin selection across requests. The
package caches a plugin-bound native prepared protocol by buffer identity,
avoiding protobuf decoding and deterministic index construction after the
first call. A byte snapshot detects buffer mutation and rebuilds the prepared
entry so cached rendering cannot return stale protocol content. All render APIs
use this prepared path; incompatible older addons fail instead of silently
falling back to per-request decoding.

### `renderStream(protocol: Buffer, state: object | string, onChunk: (html: string) => void): void`

Renders with streaming output. Internal handler writes are coalesced around a
16 KiB target before the callback crosses into JavaScript.

```js
renderStream(protocol, state, (chunk) => {
  response.write(chunk);
});
```

### `buildAndRender(options: BuildOptions, state: object | string): string`

Convenience function that builds and renders in a single call.

```js
const html = buildAndRender({ appDir: "./src" }, { name: "WebUI" });
```

### `inspect(protocol: Buffer): string`

Returns a JSON representation of the protocol for debugging.

```js
const json = inspect(protocol);
console.log(JSON.parse(json));
```

### `renderPartial(protocol: Buffer, stateJson: string, entryId: string, requestPath: string, inventoryHex: string): string`

Produces a JSON partial response for client-side navigation, including state, template metadata, condition closures, and route chain.

### `renderComponentTemplates(protocol: Buffer, componentTags: string[], inventoryHex: string): string`

Renders templates and styles for on-demand component loading (used by `Router.ensureLoaded()`). Returns a JSON string with `templateStyles`, `templates`, `templateFunctions`, and `inventory`. Uses the same inventory bitfield as partial navigation to avoid sending duplicates.

```js
const json = renderComponentTemplates(protocol, ["settings-dialog"], inventoryHex);
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
