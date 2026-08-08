# WebUI Native Node Module Handler

The `@microsoft/webui` npm package provides high-performance server-side
rendering for Node.js, Bun, and Deno. It uses a native addon with a canonical
UTF-8 `Buffer` path plus batched callbacks for streaming responses.

## Installation

```bash
npm install @microsoft/webui
```

## Examples

<webui-press-tabs>
<webui-press-tab slot="tab" active>Node.js</webui-press-tab>
<webui-press-tab slot="tab">Bun</webui-press-tab>
<webui-press-tab slot="tab">Deno</webui-press-tab>
<webui-press-tab-panel active>

```js
import { createServer } from 'node:http';
import { readFileSync } from 'node:fs';
import { Protocol } from '@microsoft/webui';

const protocol = new Protocol(
  readFileSync('./dist/protocol.bin'),
  { plugin: 'webui' },
);

const server = createServer((req, res) => {
  res.writeHead(200, { 'Content-Type': 'text/html' });
  protocol.renderStream(
    { title: 'Home' },
    (chunk) => res.write(chunk),
    { entry: 'index.html', requestPath: req.url },
  );
  res.end();
});

server.listen(3000);
```

</webui-press-tab-panel>
<webui-press-tab-panel>

```ts
import { Protocol } from '@microsoft/webui';

const protocol = Bun.file('./dist/protocol.bin');
const protocolData = Buffer.from(await protocol.arrayBuffer());
const runtimeProtocol = new Protocol(protocolData);

Bun.serve({
  port: 3000,
  fetch(req) {
    const url = new URL(req.url);
    const html = runtimeProtocol.render({ title: 'Home' }, {
      entry: 'index.html',
      requestPath: url.pathname,
    });
    return new Response(html, {
      headers: { 'Content-Type': 'text/html' },
    });
  },
});
```

</webui-press-tab-panel>
<webui-press-tab-panel>

```ts
import { Protocol } from '@microsoft/webui';

const protocol = Deno.readFileSync('./dist/protocol.bin');
const protocolData = Buffer.from(protocol);
const runtimeProtocol = new Protocol(protocolData);

Deno.serve({ port: 3000 }, (req) => {
  const url = new URL(req.url);
  const html = runtimeProtocol.render({ title: 'Home' }, {
    entry: 'index.html',
    requestPath: url.pathname,
  });
  return new Response(html, {
    headers: { 'Content-Type': 'text/html' },
  });
});
```

</webui-press-tab-panel>
</webui-press-tabs>

## API Reference

| API | Description |
|----------|-------------|
| `build(options)` | Build templates into a protocol. Returns `{ protocol, cssFiles, componentAssetFiles, metafile?, warnings, stats }` |
| `new Protocol(protocol, options?)` | Decode and index protocol bytes once and bind the selected plugin |
| `protocol.render(state, options?)` | Render into a UTF-8 `Buffer` for direct HTTP writes |
| `protocol.renderStream(state, onChunk, options?)` | Render with callbacks coalesced around a 16 KiB target before crossing into JavaScript |
| `protocol.streamResponse(options?)` | Open a [progressive streaming session](#progressive-streaming) that returns one `Buffer` per host call |
| `protocol.renderPartial(state, entry, requestPath, inventory)` | Produce a complete partial-navigation JSON response |
| `protocol.renderComponentTemplates(tags, inventory)` | Return on-demand template payloads |
| `protocol.tokens()` | Return CSS token names in build order |
| `inspect(protocol)` | Convert protocol to JSON for debugging |

### RenderOptions

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `entry` | `string` | `"index.html"` | Fragment ID to start rendering from |
| `requestPath` | `string` | `"/"` | URL path to match routes against |
`state` accepts either an object (auto-serialized) or a pre-stringified JSON string.

### ProtocolOptions

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `plugin` | `string` | - | Handler plugin bound for the lifetime of the protocol |

## Reusing Protocol

Load `protocol.bin` once and construct one `Protocol` for the lifetime of the
server:

```js
const protocol = new Protocol(
  readFileSync('./dist/protocol.bin'),
  { plugin: 'webui' },
);

const server = createServer((req, res) => {
  const html = protocol.render(getState(req), {
    entry: 'index.html',
    requestPath: req.url,
  });
  res.end(html);
});
```

`Protocol` owns the decoded native state, deterministic index, and template
metadata cache. The source `Buffer` can be released or reused after
construction. The package has no hidden `WeakMap`, protocol-sized mutation
snapshot, or render path that accepts protocol bytes on every request.

`protocol.render()` returns a UTF-8 `Buffer` so the native allocation can be
passed directly to `response.end()`. Call `.toString('utf8')` only when
JavaScript string operations are required. Use `protocol.renderStream()` when
the HTTP integration can make progress from callbacks; callbacks are batched
rather than invoked for every internal handler write. The callback is
synchronous and its return value is ignored. A `false` result from
`response.write()` cannot pause native rendering or wait for `drain`, so this
API does not provide transport backpressure. Callback exceptions abort the
render immediately and propagate to the caller.

### BuildOptions

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `appDir` | `string` | - | Path to app folder |
| `entry` | `string` | `"index.html"` | Entry file |
| `css` | `"link" \| "style" \| "module"` | `"link"` | CSS delivery strategy |
| `plugin` | `string` | - | Parser plugin name (see [Plugins](/guide/concepts/plugins/) for the available identifiers) |
| `components` | `string[]` | - | External component sources |
| `componentAssetRoots` | `string[]` | - | Root component tags emitted as static `.webui.js` ESM assets |
| `metafile` | `boolean` | `false` | Generate and return an esbuild-compatible component asset graph |
| `projectionManifests` | `string[]` | - | Projection manifest paths, merged with strict scripted-component coverage |
| `projectionManifestObjects` | `{ path: string; manifest: unknown }[]` | - | Already-transported manifests with logical paths anchoring `root` and stale checks |
| `cssFileNameTemplate` | `string` | `"[name].[ext]"` | Emitted asset filename template for Link-mode CSS and component assets. Tokens: `[name]`, `[hash]`, `[ext]` |
| `cssPublicBase` | `string` | - | Public URL/path prefix for Link-mode CSS hrefs |
| `legalComments` | `"inline" \| "none"` | `"inline"` | Preserve legal CSS comments inline, or strip all comments |
| `theme` | `string` | - | Design token theme JSON path or npm package name. Missing required CSS tokens fail the build (literal `var()` fallbacks are exempt) |

Every unwrapped component uses Light DOM. A component uses Shadow only when its
complete template is a sole top-level `<template shadowrootmode="open">`.
Native `<slot>` is Shadow-only.

```js
const result = build({
  appDir: './src',
  plugin: 'webui',
  projectionManifests: ['./dist/webui-projection.json'],
});
```

When `componentAssetRoots` contains multiple roots, the build returns a version
2 asset graph in `componentAssetFiles`: entry-reachable dependencies stay
external, single-root dependencies stay inline, and dependencies with the same
multi-root consumer set are emitted once as shared chunks. Asset-only records
are removed from `result.protocol`. Component assets cannot be combined with
`<route>`.

Set `metafile: true` to receive
`result.metafile`. The JSON uses esbuild's `inputs`/`outputs`
schema, root `entryPoint` records, and `dynamic-import` edges, so it can be
opened directly in an esbuild bundle analyzer.

Manifest inputs are build-time only. The returned protocol is self-contained,
and `render()` does not load projection tooling. If no manifest is supplied,
the build preserves full state.

The Node API requires the platform-specific native addon. Addon resolution and
loading errors are returned directly and never trigger a CLI subprocess. Use
the `webui` CLI explicitly when a filesystem-oriented build is preferred.

### BuildStats

| Field | Type | Description |
|-------|------|-------------|
| `durationMs` | `number` | Build time in milliseconds |
| `fragmentCount` | `number` | Total fragments |
| `componentCount` | `number` | Components registered |
| `cssFileCount` | `number` | CSS files produced |
| `protocolSizeBytes` | `number` | Protocol binary size |
| `tokenCount` | `number` | CSS tokens discovered |

## Progressive Streaming

`renderStream()` is push-based: the native renderer decides when your callback
runs, so a `false` result from `response.write()` cannot pause it. That is fine
for whole-document rendering, but it cannot express a response your server paces.

`protocol.streamResponse()` inverts that. It opens a **session** whose methods
return the bytes they produced, so your server owns the socket, the write order,
and the backpressure contract:

```js
import { once } from 'node:events';

const session = protocol.streamResponse({ entry: 'index.html', requestPath: '/' });

// Authored boundary names resolve to integer handles once, outside the loop.
const status = session.boundary('job-status');
const rows = session.boundary('rows');

res.writeHead(200, {
  'Content-Type': 'text/html; charset=utf-8',
  'X-Accel-Buffering': 'no',
});

await write(res, session.writeShell(baseState));
await write(res, session.writeBoundary(status, statusState, 'updatable'));
await write(res, session.writeBoundary(rows, await loadRows()));

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

That `write` helper is the entire transport integration, which is why
the same session drops into Express, Fastify, Hapi, or a raw socket unchanged.

The page's entry template must declare
[`<boundary>` directives](/guide/concepts/directives/boundary); `boundaryCount`
reports how many it has.

### StreamingSession

| Member | Description |
|--------|-------------|
| `boundary(name)` | Resolve an authored boundary name to its integer handle. Throws with the valid names and a "did you mean …?" suggestion on a typo. |
| `boundaryCount` | Number of boundaries the entry declares |
| `finished` | Whether `finish()` has been called |
| `writeShell(state)` | Bytes for the document prefix through the first semantic flush |
| `writeBoundary(id, state, mode?)` | Bytes for one boundary's markup, metadata delta, and checkpoint. `mode` is `"final"` (default) or `"updatable"` |
| `update(id, state)` | Bytes for a projected state patch to a boundary committed as `"updatable"` |
| `finish(state)` | Bytes for the tail checkpoint, terminal record, and document suffix |

Ordering is enforced: the shell first, boundaries in declaration order, updates
only to updatable boundaries already committed, and `finish()` last. A rejected
call throws and leaves the session usable, so invalid state does not cost you the
response. Sessions are independent, so hold one per in-flight request.

**Runnable example.** `examples/integration/node/streaming-server.js` is a
complete `node:http` server built on this API, with no sidecar process.
