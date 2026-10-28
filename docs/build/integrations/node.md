---
title: "Node.js and Bun"
description: "Render with the native npm binding, reuse protocols and prepared state, and choose buffered or host-paced streaming responses."
---

<a id="webui-native-node-module-handler"></a>
<a id="nodejs-bun-and-deno"></a>

# Node.js and Bun

`@microsoft/webui` exposes the Rust renderer through a platform-specific native
addon for Node.js and Bun. Load `protocol.bin` once, reuse the `Protocol`,
and pass JSON state for each render. Buffered output is a UTF-8 `Buffer`.

## Installation

```bash
npm install @microsoft/webui
```

Keep optional dependencies enabled so npm can install the matching native
package. This API needs native-addon support; it is not a browser or
JavaScript-only edge-runtime package.

## Examples

<webui-press-tabs>
<webui-press-tab slot="tab" active>Node.js</webui-press-tab>
<webui-press-tab slot="tab">Bun</webui-press-tab>
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
</webui-press-tabs>

## API Reference

| API | Description |
|----------|-------------|
| `build(options)` | Build templates into a protocol. Returns `{ protocol, cssFiles, componentAssetFiles, metafile?, warnings, stats }` |
| `new Protocol(protocol, options?)` | Decode and index protocol bytes once and bind the selected plugin |
| `protocol.render(state, options?)` | Render into a UTF-8 `Buffer` for direct HTTP writes |
| `protocol.prepareState(state)` | Parse an immutable, process-local native state snapshot once |
| `protocol.renderPrepared(state, options?)` | Render a prepared snapshot without serializing or parsing it again |
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

`Protocol` owns the loaded native artifact. The source `Buffer` can be released
or reused after construction; subsequent renders use the loaded object.

`protocol.render()` returns a UTF-8 `Buffer` so the native allocation can be
passed directly to `response.end()`. Call `.toString('utf8')` only when
JavaScript string operations are required. Use `protocol.renderStream()` when
the HTTP integration can make progress from callbacks; callbacks are batched
rather than invoked for every internal handler write. The callback is
synchronous and its return value is ignored. A `false` result from
`response.write()` cannot pause native rendering or wait for `drain`, so this
API does not provide transport backpressure. Callback exceptions abort the
render immediately and propagate to the caller.

### Reusing immutable state

When one state snapshot is rendered more than once, prepare it alongside the
protocol:

```js
const cachedState = protocol.prepareState(await loadCatalogState());

const server = createServer((_req, res) => {
  res.end(protocol.renderPrepared(cachedState));
});
```

Preparation performs serialization and native JSON parsing once.
`renderPrepared()` reuses the immutable native tree and produces the same bytes
as `render()` for matching options. Later mutations to the source object are not
visible; create a new prepared state when request data changes.

The opaque handle is process-local and cannot be serialized. It retains the
native state tree until JavaScript garbage collection releases the handle, so
this API trades resident native memory for lower repeated-render CPU and
allocation pressure. Ordinary per-request state should continue to use
`render()`.

### BuildOptions

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `appDir` | `string` | - | Path to app folder |
| `entry` | `string` | `"index.html"` | Entry file |
| `css` | `"link" \| "style" \| "module"` | `"link"` | CSS delivery strategy |
| `dom` | `"shadow" \| "light"` | `"shadow"` | Fallback for unwrapped components; Light builds retain authored Shadow islands |
| `cssBundle` | `boolean` | `false` | Merge component stylesheets into shared chunks. Composes with `css`; rejected with `css: "module"` |
| `plugin` | `string` | - | Parser plugin name (see [Plugins](/reference/plugins/) for the available identifiers) |
| `components` | `string[]` | - | External component sources |
| `componentAssetRoots` | `string[]` | - | Root component tags emitted as static `.webui.js` ESM assets |
| `metafile` | `boolean` | `false` | Generate and return an esbuild-compatible component asset graph |
| `projectionManifests` | `string[]` | - | Projection manifest paths, merged with strict scripted-component coverage |
| `projectionManifestObjects` | `{ path: string; manifest: unknown }[]` | - | Already-transported manifests with logical paths anchoring `root` and stale checks |
| `cssFileNameTemplate` | `string` | `"[name].[ext]"` | Emitted filename template for Link-mode CSS. Tokens: `[name]`, `[hash]`, `[ext]` |
| `cssPublicBase` | `string` | - | Public URL/path prefix for Link-mode CSS hrefs |
| `legalComments` | `"inline" \| "none"` | `"inline"` | Preserve legal CSS comments inline, or strip all comments |
| `theme` | `string` | - | Design token theme JSON path or npm package name. Missing required CSS tokens fail the build (literal `var()` fallbacks are exempt) |

Unwrapped components default to generated open Shadow roots. Set `dom: "light"`
to make them global Light DOM; authored sole open Shadow roots remain Shadow.
Light CSS uses ordinary selectors, and `:host`, `:host-context`, and
`::slotted` fail with `unsupported-light-css`.

```js
const result = build({
  appDir: './src',
  plugin: 'webui',
  projectionManifests: ['./dist/webui-projection.json'],
});
```

When `componentAssetRoots` is set, `componentAssetFiles` contains stable version
4 ESM inputs: one module per asset-owned component plus a thin root entry with
static imports for its closure. Entry-reachable dependencies stay external and
asset-only records are removed from `result.protocol`. Pass these inputs to the
application bundler, which owns final splitting, hashes, public paths, caching,
and delivery. Component assets cannot be combined with `<route>`.

Set `metafile: true` to receive
`result.metafile`. The JSON uses esbuild's `inputs`/`outputs`
schema, root `entryPoint` records, and static `import-statement` edges, so it can
be opened directly in an esbuild bundle analyzer.

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

`renderStream()` calls a synchronous callback and ignores its return value.
It cannot wait for `response.write()` backpressure. Use a progressive session
when the host needs to pace delivery or wait for boundary data.

`protocol.streamResponse()` inverts that. It opens a **session** whose methods
return bytes, so your server owns the socket and backpressure:

```js
const session = protocol.streamResponse({
  entry: 'index.html',
  requestPath: '/',
});

res.writeHead(200, {
  'Content-Type': 'text/html; charset=utf-8',
  'X-Accel-Buffering': 'no',
});

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

The same shape works behind Express, Fastify, Hapi, or a raw socket. Boundaries
are discovered at runtime through entries, reusable components, conditions, and
the selected route. A boundary-bearing subtree under `<for>` fails the build
with `boundary-in-repeat`; a whole `<for>` may sit inside one boundary.

For a coordinator-only head script, follow the
[bundler-independent delivery contract](/build/hydration#separate-coordinator-and-application-assets).
Configure script URLs through your existing asset handoff; the native
`StreamingSession` does not depend on a bundler or read asset manifests.
The application explicitly imports `@microsoft/webui-framework/streaming.js`
in its own early entry and registers that entry with its bundler.
Application registrations may load later, but are required before their
components become interactive.

### StreamingSession

| Member | Description |
|--------|-------------|
| `start(state)` | Return `{ bytes, done, boundary? }` through the first occurrence or terminal |
| `resume(instanceId, state, mode?)` | Return only the pending occurrence's bytes through its checkpoint |
| `advance()` | Return following parent bytes through the next occurrence or terminal |
| `update(instanceId, patch)` | Return projected state bytes for a committed updatable occurrence |

A descriptor contains `instanceId`, `declarationId`, `owner`, `name`, and an
optional string or numeric `key`. Use those fields to load state, then pass
`instanceId` back to `resume`. A descriptor means call `resume`; no descriptor
with `done: false` means call `advance`; `done: true` means complete.

Write and flush the boundary-only `resume` result before waiting for more data.
`advance` carries the following parent content; the final step includes the
document tail. `mode` is `"final"` by default or `"updatable"`.

An update never inserts markup or reruns hydration:

```js
const patch = session.update(searchInstanceId, { query: 'webui' });
await write(res, patch);
```

An update may be written between the occurrence's `resume` and `advance`.
Sessions are single-driver and independent. Hold one per in-flight request and
stop driving it after a rendering or transport failure.
