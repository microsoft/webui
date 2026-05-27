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
  cssFileNameTemplate: "[name]-[hash].[ext]", // Link-mode CSS filename template
  cssPublicBase: "https://cdn.example.com/assets", // Optional CDN/public href base
  outDir: "./dist",      // Output directory for CLI fallback
});

// result.protocol  - Buffer containing the compiled protocol
// result.cssFiles  - Array of [filename, content, ...] pairs
// result.stats     - { durationMs, fragmentCount, componentCount, cssFileCount, protocolSizeBytes, tokenCount }
```

### `render(protocol: Buffer, state: object | string): string`

Renders a compiled protocol with state data and returns the full HTML string.

```js
const html = render(protocol, { title: "Hello", show: true });
```

### `renderStream(protocol: Buffer, state: object | string, onChunk: (html: string) => void): void`

Renders with streaming output - each HTML fragment is passed to the callback as it is produced.

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

Produces a JSON partial response for client-side navigation, including state, templates, and route chain.

### `renderComponentTemplates(protocol: Buffer, componentTags: string[], inventoryHex: string): string`

Renders templates and styles for on-demand component loading (used by `Router.ensureLoaded()`). Returns a JSON string with `templateStyles`, `templates`, and `inventory`. Uses the same inventory bitfield as partial navigation to avoid sending duplicates.

```js
const json = renderComponentTemplates(protocol, ["settings-dialog"], inventoryHex);
const { templates, templateStyles, inventory } = JSON.parse(json);
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
