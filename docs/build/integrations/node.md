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

<webui-press-tabs sync="package-manager" storage="local">
<webui-press-tab slot="tab" value="npm" active>npm</webui-press-tab>
<webui-press-tab slot="tab" value="pnpm">pnpm</webui-press-tab>
<webui-press-tab slot="tab" value="yarn">Yarn</webui-press-tab>
<webui-press-tab slot="tab" value="bun">Bun</webui-press-tab>
<webui-press-tab-panel active>

```bash
npm install @microsoft/webui
```

</webui-press-tab-panel>
<webui-press-tab-panel>

```bash
pnpm add @microsoft/webui
```

</webui-press-tab-panel>
<webui-press-tab-panel>

```bash
yarn add @microsoft/webui
```

</webui-press-tab-panel>
<webui-press-tab-panel>

```bash
bun add @microsoft/webui
```

</webui-press-tab-panel>
</webui-press-tabs>

Keep optional dependencies enabled so your manager can install the matching native
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
| `build(options)` | Build templates into a protocol. Returns `{ protocol, cssFiles, componentAssetFiles, lazyPackFiles, lazyScriptsModule?, metafile?, warnings, stats }` |
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

## Lazy template packs

[Lazy bodies](/learn/lazy) need no request-time API. `build()` returns
their packs in `lazyPackFiles`; `webui build` writes them for you. Publish each
pack at its `path`, under `_webui/packs/` beside the page, as an immutable
static file. When the build has lazy regions, `lazyScriptsModule` is the
generated `lazy.webui.js`, and its `declaration` is the `lazy.webui.d.ts` that
gives TypeScript entries its types without `allowJs`. Write both to the
directory you passed as `modulesOut` and import the module from your entry
module, so your bundler loads the code of the components each region
introduces:

```typescript
const result = build({ appDir: './src', plugin: 'webui', modulesOut: './.webui' });
for (const pack of result.lazyPackFiles) {
  writeFileSync(join('dist', pack.path), pack.content);
}
const lazy = result.lazyScriptsModule;
if (lazy) {
  writeFileSync(join('.webui', lazy.fileName), lazy.content);
  writeFileSync(join('.webui', lazy.declaration.fileName), lazy.declaration.content);
}
```

Relative module specifiers in `lazy.webui.js` start at `modulesOut`, or at
`appDir` when you omit it. A region lists only the components that neither its
owner nor the page at startup already loads. `componentModules` maps a tag to a
module specifier for a component without a usable `.ts` or `.js` sibling; see
[Lazy region scripts](/reference/cli/#webui-build).

Each `lazyPackFiles` item reports its pack:

| Field | Type | Description |
|-------|------|-------------|
| `fileName` | `string` | Content-addressed file name |
| `content` | `string` | JSON pack content |
| `path` | `string` | `_webui/packs/<fileName>`, relative to the asset base |
| `region` | `string` | Region identity, such as `app-shell::lazy:@0` |
| `owner` | `string` | Component whose template declares the region |
| `trigger` | `"intent" \| "idle" \| "eager" \| "demand"` | The region's `lazy` value |
| `hash` | `string` | Content hash from the file name |
| `bytes` | `number` | Size of `content` in bytes |
| `brotliBytes` | `number` | Brotli size at quality 11; present only when `metafile: true` |
| `scripts` | `string[]` | Module specifiers `lazy.webui.js` loads with the region |

Templates reference packs by the path `_webui/packs/<file>`, relative to the
asset base. To publish the output elsewhere, such as a CDN, pass `assetBase`
(ending in `/`) to `build()`, or set it per request in the render state:

```typescript
protocol.render({ ...state, $webui: { assetBase: 'https://cdn.example.com/b123/' } });
```

The request value overrides `assetBase`. Rendering prefixes stylesheet links
with the base and passes it to the browser, which resolves pack and stylesheet
paths against it. Module preload hints keep their paths, because they name your
own scripts. An invalid request value throws before any output is produced.

### Bundler public path

The rendered document head carries the response's base as
`<meta name="webui-asset-base" content="...">`, so your entry can hand it to
your bundler before the WebUI runtime starts. The region scripts that
`lazy.webui.js` imports then load from the same base. With rspack or webpack,
run this first in your entry module:

```js
__webpack_public_path__ = document.querySelector('meta[name="webui-asset-base"]')?.content ?? __webpack_public_path__;
```

Static imports run before the statements of the module that imports them, so
if your entry has static imports, put the line in its own module and import
that module first.

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
| `metafile` | `boolean` | `false` | Return the build metafile: the component asset graph and the `lazyPacks` report. Also measures `brotliBytes` for each lazy pack |
| `componentModules` | `Record<string, string>` | - | Module specifiers keyed by custom element tag, for components without a usable `.ts` or `.js` sibling. Written verbatim in `lazy.webui.js` |
| `modulesOut` | `string` | `appDir` | Directory you write generated modules to; relative specifiers in `lazy.webui.js` start here |
| `projectionManifests` | `string[]` | - | Projection manifest paths, merged with strict scripted-component coverage |
| `projectionManifestObjects` | `{ path: string; manifest: unknown }[]` | - | Already-transported manifests with logical paths anchoring `root` and stale checks |
| `cssFileNameTemplate` | `string` | `"[name].[ext]"` | Emitted filename template for Link-mode CSS. Tokens: `[name]`, `[hash]`, `[ext]` |
| `assetBase` | `string` | - | Public URL or path ending in `/` that maps to the build output. Stored in the protocol; requests override it with `$webui.assetBase`. See [Asset base](/reference/cli/) |
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
`result.metafile` when the build emits component assets or lazy packs. The JSON
uses esbuild's `inputs`/`outputs` schema, root `entryPoint` records, and static
`import-statement` edges, so it can be opened directly in an esbuild bundle
analyzer. Its `lazyPacks` array lists every lazy pack, sorted by path, with the
fields above except `fileName` and `content`; see
[Build metafile](/reference/cli/#webui-build).

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
