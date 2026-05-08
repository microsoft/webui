# WebUI WebAssembly Handler

The WebUI WASM handler compiles the full rendering pipeline to WebAssembly via `wasm-bindgen`, enabling client-side rendering directly in the browser. It powers the interactive [Playground](/playground/) in the documentation site.

## How It Works

The WASM module includes the real `webui-parser` and `webui-handler` - the same code used by the CLI and the Rust handler. This means templates parsed in the browser produce identical output to server-side rendering.

Two modes of operation are available:

1. **`render`** - Takes a pre-built protocol (JSON) + state and renders HTML
2. **`build_and_render`** - Takes virtual files + state, parses and renders in one call

## Building the WASM Module

```bash
cargo xtask build-wasm
```

The output is committed to the repository under `docs/` for use by the playground. Most developers don't need to rebuild it - only rebuild when you change Rust code in the core crates.

## API Reference

### `render(protocolJson, stateJson, plugin?)`

Render a pre-built WebUI protocol with state data.

```js
import init, { render } from './webui_wasm.js';

await init();

const protocolJson = '{"fragments": {...}}';
const stateJson = '{"title": "Hello"}';

const html = render(protocolJson, stateJson);
```

**Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `protocolJson` | `string` | JSON-serialized `WebUIProtocol` |
| `stateJson` | `string` | JSON string with the render state |
| `plugin` | `string \| undefined` | Parser plugin name (see [Plugins](/guide/concepts/plugins/) for the available identifiers) |

**Returns:** Rendered HTML string. Throws on error.

### `build_and_render(files, stateJson, entry)`

Parse virtual files and render in a single call. This is the primary API for the playground, where no filesystem is available.

```js
import init, { build_and_render } from './webui_wasm.js';

await init();

const files = {
  'index.html': '<h1>Hello, {{name}}!</h1>',
  'my-card.html': '<div class="card"><slot></slot></div>',
  'my-card.css': '.card { border: 1px solid #ccc; }'
};
const stateJson = '{"name": "WebUI"}';

const html = build_and_render(files, stateJson, 'index.html');
// → '<h1>Hello, WebUI!</h1>'
```

**Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `files` | `Record<string, string>` | Map of filenames to content |
| `stateJson` | `string` | JSON string with the render state |
| `entry` | `string` | Entry HTML filename (e.g., `"index.html"`) |

**Returns:** Rendered HTML string. Throws on error.

**Component discovery:** HTML files with a hyphen in the name are automatically registered as components (e.g., `my-card.html` → `<my-card>`). Matching `.css` files are paired and inlined via `<style>` tags.

### `build_protocol(files, entry)`

Parse virtual files into a WebUI protocol without rendering. Returns the serialized protocol as a JSON string.

```js
import init, { build_protocol } from './webui_wasm.js';

await init();

const files = {
  'index.html': '<h1>{{title}}</h1>'
};

const protocolJson = build_protocol(files, 'index.html');
// Use with render() later
```

**Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `files` | `Record<string, string>` | Map of filenames to content |
| `entry` | `string` | Entry HTML filename |

**Returns:** JSON string of the `WebUIProtocol`.

## Using Plugins

Pass a plugin identifier (see [Plugins](/guide/concepts/plugins/)) as the `plugin` parameter to enable hydration markers:

```js
const html = render(protocolJson, stateJson, 'webui');
```

The plugin runs the same parser/handler code used by the CLI; output is byte-identical to a server-side render with the same plugin selected.

## Playground Integration

The documentation playground uses `build_and_render` to provide a live editing experience:

1. User edits template HTML, component files, and state JSON in the browser
2. On each change, `build_and_render` is called with the virtual files
3. The rendered HTML is displayed in a preview pane

This provides instant feedback with the exact same parser and handler used in production builds.

## Differences from Server-Side Rendering

| Aspect | Server (CLI / Rust / Node) | WASM (Browser) |
|--------|---------------------------|----------------|
| Protocol format | Protobuf binary | JSON |
| CSS strategy | Link (default) or style | Always inline |
| File I/O | Filesystem | Virtual file map |
| Streaming | Yes (callback-based) | No (returns full string) |
| Plugin support | Yes | Yes |
