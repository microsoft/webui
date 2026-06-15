# WebUI WebAssembly

WebUI provides browser-ready WebAssembly bindings through `wasm-bindgen`. The bindings are built as three variants so you can choose only the parser, only the handler, or the combined playground bundle.

## Variants

| Variant | Import path | Exports | Use when |
|---------|-------------|---------|----------|
| Handler | `wasm/handler/webui_wasm_handler.js` | `render`, `render_partial`, `protocol_tokens`, `render_component_templates` | You already have protocol bytes and only need rendering |
| Parser | `wasm/parser/webui_wasm_parser.js` | `build_protocol` | You need to compile virtual browser files into protocol bytes |
| All | `wasm/all/webui_wasm_all.js` | Parser and handler exports | You need both sides in one module, such as the docs playground |

The handler-only bundle excludes `webui-parser`, and the parser-only bundle excludes `webui-handler`. The combined bundle keeps the previous playground behavior.

## Building the WASM bundles

```bash
cargo xtask build-wasm
```

The output is generated under `docs/.webui-press/public/wasm/` for the documentation playground and release staging. Rebuild it when Rust code in `webui-wasm`, `webui-parser`, `webui-handler`, or `webui-protocol` changes.

## Handler-only API

Use the handler bundle when the protocol was built elsewhere and loaded as protobuf bytes in the browser.

```js
import init, { render } from './wasm/handler/webui_wasm_handler.js';

await init();

const protocolBytes = new Uint8Array(await (await fetch('/protocol.bin')).arrayBuffer());
const stateJson = '{"title": "Hello"}';
let html = '';

render(protocolBytes, stateJson, (chunk) => {
  html += chunk;
}, { entry: 'index.html', requestPath: '/', plugin: 'webui' });
```

### `render(protocolBytes, stateJson, onChunk, options?)`

Render a pre-built WebUI protocol with state data.

| Parameter | Type | Description |
|-----------|------|-------------|
| `protocolBytes` | `Uint8Array` | Protobuf-serialized `WebUIProtocol`, such as `protocol.bin` |
| `stateJson` | `string` | JSON string with render state |
| `onChunk` | `(html: string) => void` | Callback invoked with each rendered HTML fragment |
| `options.entry` | `string \| undefined` | Entry fragment ID, defaults to `index.html` |
| `options.requestPath` | `string \| undefined` | Request path used for route matching, defaults to `/` |
| `options.plugin` | `string \| undefined` | Handler plugin name, such as `webui`, `fast-v3`, `fast-v2`, or `fast` |

Returns nothing on success. Throws on protocol, state, plugin, callback, or render errors.

### Additional handler exports

| Export | Description |
|--------|-------------|
| `render_partial(protocolBytes, stateJson, entry, requestPath, inventoryHex)` | Returns the JSON partial-navigation response with `state` included |
| `protocol_tokens(protocolBytes)` | Returns the protocol CSS token names as a JavaScript array |
| `render_component_templates(protocolBytes, componentTagsJson, inventoryHex)` | Returns template and style payloads for requested components |

## Parser-only API

Use the parser bundle when browser code needs to compile an in-memory file map into protocol bytes.

```js
import init, { build_protocol } from './wasm/parser/webui_wasm_parser.js';

await init();

const files = {
  'index.html': '<h1>{{title}}</h1>',
  'my-card.html': '<p><slot></slot></p>',
  'my-card.css': 'p { color: red; }',
};

const protocolBytes = build_protocol(files, 'index.html');
```

### `build_protocol(files, entry)`

Parse virtual files into a WebUI protocol without rendering.

| Parameter | Type | Description |
|-----------|------|-------------|
| `files` | `Record<string, string>` | Map of filenames to content |
| `entry` | `string` | Entry HTML filename |

Returns protobuf-serialized `WebUIProtocol` as a `Uint8Array`. Throws on missing entry files, invalid templates, invalid component authoring, or protocol serialization errors.

Component discovery follows the virtual file map convention: HTML files with a hyphen in the name are registered as components, such as `my-card.html` for `<my-card>`. Matching `.css` files are paired and inlined with `CssStrategy::Style`.

## Combined API

Use the combined bundle when you want parser and handler exports from one module.

```js
import init, { build_protocol, render } from './wasm/all/webui_wasm_all.js';

await init();

const protocolBytes = build_protocol(files, 'index.html');
let html = '';
render(protocolBytes, stateJson, (chunk) => {
  html += chunk;
}, { entry: 'index.html', requestPath: '/' });
```

The documentation playground imports this combined bundle and currently uses `build_protocol` followed by `render` so it can measure compile and render time separately.

## Differences from server-side rendering

| Aspect | Server (CLI / Rust / Node) | WASM (Browser) |
|--------|---------------------------|----------------|
| Protocol format | Protobuf binary | Protobuf bytes |
| CSS strategy | Link by default, Style or Module when configured | Style for virtual file builds |
| File I/O | Filesystem and component discovery sources | Virtual file map |
| Streaming | Supported by native handlers | `render()` calls a JavaScript chunk callback |
| Bundle choice | Native crates/addons | Handler-only, parser-only, or combined WASM |
