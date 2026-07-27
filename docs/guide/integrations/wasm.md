# WebUI WebAssembly

WebUI provides browser-ready WebAssembly bindings through `wasm-bindgen`. The bindings are built as three variants so you can choose only the parser, only the handler, or the combined playground bundle.

## Variants

| Variant | Import path | Exports | Use when |
|---------|-------------|---------|----------|
| Handler | `wasm/handler/webui_wasm_handler.js` | `Protocol` | You already have protocol bytes and only need rendering |
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
import init, { Protocol } from './wasm/handler/webui_wasm_handler.js';

await init();

const protocolBytes = new Uint8Array(await (await fetch('/protocol.bin')).arrayBuffer());
const protocol = new Protocol(protocolBytes, 'webui');
const html = protocol.render(
  '{"title": "Hello"}',
  { entry: 'index.html', requestPath: '/' },
);
```

Keep the `Protocol` instance alive across renders. It decodes protobuf, builds
deterministic indices, and binds the plugin once.

### `Protocol`

| Method | Description |
|--------|-------------|
| `render(stateJson, options?)` | Return complete rendered HTML as a string |
| `renderStream(stateJson, onChunk, options?)` | Invoke callbacks coalesced around a 16 KiB target |
| `renderPartial(stateJson, entry, requestPath, inventoryHex)` | Return a complete JSON partial response with active-route projected state |
| `renderComponentTemplates(componentTags, inventoryHex)` | Return requested template payloads and updated inventory |
| `tokens()` | Return CSS token names in build order |

For a complete static/CDN service worker example using this callback to write a
`ReadableStream` and mirror `--theme` token injection in the browser, see
[Serverless Architecture](/guide/serverless-architecture).

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

const protocolBytes = build_protocol(
  files,
  'index.html',
  [projectionManifest],
);
```

### `build_protocol(files, entry, projectionManifests?)`

Parse virtual files into a WebUI protocol without rendering.

| Parameter | Type | Description |
|-----------|------|-------------|
| `files` | `Record<string, string>` | Map of filenames to content |
| `entry` | `string` | Entry HTML filename |
| `projectionManifests` | `object[]` | Optional bundler manifest fragments |

Returns protobuf-serialized `WebUIProtocol` as a `Uint8Array`. Throws on missing entry files, invalid templates, invalid component authoring, or protocol serialization errors.

Without manifests, initial and scripted navigation state remain full. With
manifests, WASM applies the shared schema/build-ID validation, fragment merge,
and strict coverage rules. Because virtual WASM builds have no filesystem, they
cannot repeat disk stale-file checks and never analyze JavaScript.

Component discovery follows the virtual file map convention: HTML files with a hyphen in the name are registered as components, such as `my-card.html` for `<my-card>`. Matching `.css` files are paired and inlined with `CssStrategy::Style`.

## Combined API

Use the combined bundle when you want parser and handler exports from one module.

```js
import init, { build_protocol, Protocol } from './wasm/all/webui_wasm_all.js';

await init();

const protocolBytes = build_protocol(files, 'index.html');
let html = '';
const protocol = new Protocol(protocolBytes);
protocol.renderStream(stateJson, (chunk) => {
  html += chunk;
}, { entry: 'index.html', requestPath: '/' });
```

The documentation playground imports this combined bundle and uses
`build_protocol` followed by `new Protocol(...)` so it can measure compile and
render time separately.

## Differences from server-side rendering

| Aspect | Server (CLI / Rust / Node) | WASM (Browser) |
|--------|---------------------------|----------------|
| Protocol format | Protobuf binary | Protobuf bytes |
| CSS strategy | Link by default, Style or Module when configured | Style for virtual file builds |
| File I/O | Filesystem and component discovery sources | Virtual file map |
| Streaming | Supported by native handlers | `Protocol.renderStream()` calls a batched JavaScript callback |
| Bundle choice | Native crates/addons | Handler-only, parser-only, or combined WASM |
