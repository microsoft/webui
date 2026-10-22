---
title: "WebAssembly"
description: "Build browser WASM bundles and use the parser, loaded Protocol, or progressive streaming session without native host bindings."
---

<a id="webui-webassembly"></a>

# WebAssembly

Build WebUI's `wasm-bindgen` bundles for applications that compile or render in
the browser. Choose the handler for precompiled protocols, the parser for
virtual source files, or both for a playground. These are generated WASM assets,
not the native `@microsoft/webui` npm API.

Ordinary native SSR does not require browser WASM or a browser download of
`protocol.bin`. This integration deliberately moves that work to the client.

## Variants

| Variant | Import path | Exports | Use when |
|---------|-------------|---------|----------|
| Handler | `wasm/handler/webui_wasm_handler.js` | `Protocol` | You already have protocol bytes and only need rendering |
| Parser | `wasm/parser/webui_wasm_parser.js` | `build_protocol` | You need to compile virtual browser files into protocol bytes |
| All | `wasm/all/webui_wasm_all.js` | Parser and handler exports | You need both sides in one module, such as the docs playground |

Use the narrowest bundle your application needs. The handler excludes the
parser, and the parser excludes the renderer.

## Lazy template responses

The loaded `Protocol` also serves [lazy bodies](../concepts/lazy).
`renderLazyResources(owner, id, options)` returns resource JSON;
`prepareLazyResources` returns `{ json, options }` for two-phase delivery.
Pass the acknowledgement to
`renderLazyPartial(stateJson, owner, id, prepared.options)` after resolving
application state. Options use `inventory`, `styleInventory`, and `buildId`.

The host owns state loading and transport. These APIs do not navigate, create
components, or perform browser hydration. The handler-only variant can serve a
protocol built elsewhere; the parser and combined variants assign artifact
identity when building new protocols.

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

Keep the `Protocol` instance alive across renders rather than decoding the
same bytes repeatedly.

### `Protocol`

| Method | Description |
|--------|-------------|
| `render(stateJson, options?)` | Return complete rendered HTML as a string |
| `renderStream(stateJson, onChunk, options?)` | Invoke callbacks coalesced around a 16 KiB target |
| `streamResponse(entry, requestPath, options?)` | Open a progressive [`StreamingSession`](#streamingsession) returning one `Uint8Array` per call |
| `renderPartial(stateJson, entry, requestPath, inventoryHex)` | Return a complete JSON partial response with active-route projected state |
| `renderComponentTemplates(componentTags, inventoryHex)` | Return requested template payloads and updated inventory |
| `renderLazyResources(owner, id, options?)` | Resource-only JSON for a named template body |
| `prepareLazyResources(owner, id, options?)` | Resource JSON and acknowledgement for two-phase delivery |
| `renderLazyPartial(stateJson, owner, id, options?)` | Complete/terminal JSON with fresh scoped state |
| `tokens()` | Return CSS token names in build order |

For a service-worker example that drives a progressive session and prepares
theme CSS at build time, see
[Serverless Architecture](/guide/serverless-architecture).

### `StreamingSession`

`streamResponse()` opens a runtime-discovered session that maps directly onto a
`ReadableStream` controller:

```js
const session = protocol.streamResponse('index.html', '/');

const body = new ReadableStream({
  async start(controller) {
    let step = session.start(JSON.stringify(initialState));
    controller.enqueue(step.bytes);
    while (!step.done) {
      const boundary = step.boundary;
      if (boundary) {
        const state = await loadBoundaryState(
          boundary.owner,
          boundary.name,
          boundary.key,
        );
        step = session.resume(
          boundary.instanceId,
          JSON.stringify(state),
          'final',
        );
      } else {
        step = session.advance();
      }
      controller.enqueue(step.bytes);
    }
    controller.close();
  },
});
```

| Member | Description |
|--------|-------------|
| `start(stateJson)` | Return `{ bytes, done, boundary? }` through the first occurrence or terminal |
| `resume(instanceId, stateJson, mode?)` | Return only the pending occurrence's bytes through its checkpoint |
| `advance()` | Return following parent bytes through the next occurrence or terminal |
| `update(instanceId, patchJson)` | Return projected state bytes for an updatable occurrence |

Descriptors contain `instanceId`, `declarationId`, `owner`, `name`, and an
optional string or numeric `key`. A descriptor means `resume`; no descriptor
with `done: false` means `advance`; `done: true` means complete. `resume`
returns only the occurrence and checkpoint so it can be enqueued immediately.
`advance` returns the following parent or tail bytes. An update is valid
between `resume` and `advance`. The
final step's `Uint8Array` includes tail and terminal bytes. The contract is
identical on Node, Python, C, and .NET.

The example above shows step ordering, not a bounded transport adapter.
For large responses, also handle stream cancellation and consumer backpressure;
enqueuing bytes does not by itself wait for the consumer.

## Parser-only API

Use the parser bundle when browser code needs to compile an in-memory file map into protocol bytes.

```js
import init, { build_protocol } from './wasm/parser/webui_wasm_parser.js';

await init();

const files = {
  'index.html': '<h1>{{title}}</h1>',
  'my-card.html': '<template shadowrootmode="open"><p><slot></slot></p></template>',
  'my-card.css': 'p { color: red; }',
};

const protocolBytes = build_protocol(
  files,
  'index.html',
  [projectionManifest],
  'light',
);
```

### `build_protocol(files, entry, projectionManifests?, dom?)`

Parse virtual files into a WebUI protocol without rendering.

| Parameter | Type | Description |
|-----------|------|-------------|
| `files` | `Record<string, string>` | Map of filenames to content |
| `entry` | `string` | Entry HTML filename |
| `projectionManifests` | `object[]` | Optional bundler manifest fragments |
| `dom` | `"shadow" \| "light"` | Optional unwrapped-component fallback; defaults to `"shadow"` |

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
| Streaming | Supported by native handlers | `Protocol.renderStream()` calls a batched JavaScript callback, and `Protocol.streamResponse()` returns progressive chunks |
| Bundle choice | Native crates/addons | Handler-only, parser-only, or combined WASM |
