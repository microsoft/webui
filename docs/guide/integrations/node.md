# WebUI Native Node Module Handler

The `@microsoft/webui` npm package provides high-performance server-side
rendering for Node.js, Bun, and Deno. It uses a native addon with buffered
string and UTF-8 `Buffer` paths plus batched callbacks for streaming responses.

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
| `build(options)` | Build templates into a protocol. Returns `{ protocol, cssFiles, componentAssetFiles, warnings, stats }` |
| `new Protocol(protocol, options?)` | Decode and index protocol bytes once and bind the selected plugin |
| `protocol.render(state, options?)` | Render with route matching and return a JavaScript string |
| `protocol.renderBuffer(state, options?)` | Render into a UTF-8 Node.js `Buffer` for direct HTTP writes |
| `protocol.renderStream(state, onChunk, options?)` | Render with callbacks coalesced around a 16 KiB target before crossing into JavaScript |
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
  const html = protocol.renderBuffer(getState(req), {
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

Use `protocol.render()` when the complete HTML string is needed. For a
buffered HTTP response, prefer `protocol.renderBuffer()` so the native UTF-8
allocation can be passed directly to `response.end()` without first creating a
JavaScript string. Use `protocol.renderStream()` when the HTTP integration can
make progress from callbacks; callbacks are batched rather than invoked for
every internal handler write.

### BuildOptions

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `appDir` | `string` | - | Path to app folder |
| `entry` | `string` | `"index.html"` | Entry file |
| `css` | `"link" \| "style" \| "module"` | `"link"` | CSS delivery strategy |
| `dom` | `"shadow" \| "light"` | `"shadow"` | DOM strategy for component rendering |
| `plugin` | `string` | - | Parser plugin name (see [Plugins](/guide/concepts/plugins/) for the available identifiers) |
| `components` | `string[]` | - | External component sources |
| `componentAssetRoots` | `string[]` | - | Root component tags emitted as static `.webui.js` ESM assets |
| `projectionManifests` | `string[]` | - | Projection manifest paths, merged with strict scripted-component coverage |
| `projectionManifestObjects` | `{ path: string; manifest: unknown }[]` | - | Already-transported manifests with logical paths anchoring `root` and stale checks; native addon only |
| `cssFileNameTemplate` | `string` | `"[name].[ext]"` | Emitted asset filename template for Link-mode CSS and component assets. Tokens: `[name]`, `[hash]`, `[ext]` |
| `cssPublicBase` | `string` | - | Public URL/path prefix for Link-mode CSS hrefs |
| `legalComments` | `"inline" \| "none"` | `"inline"` | Preserve legal CSS comments inline, or strip all comments |
| `theme` | `string` | - | Design token theme JSON path or npm package name. Missing required CSS tokens fail the build (literal `var()` fallbacks are exempt) |

```js
const result = build({
  appDir: './src',
  plugin: 'webui',
  projectionManifests: ['./dist/webui-projection.json'],
});
```

Manifest inputs are build-time only. The returned protocol is self-contained,
and `render()` does not load projection tooling. If no manifest is supplied,
the build preserves full state. Inline objects require the native addon; the
CLI fallback accepts manifest paths only.

### BuildStats

| Field | Type | Description |
|-------|------|-------------|
| `durationMs` | `number` | Build time in milliseconds |
| `fragmentCount` | `number` | Total fragments |
| `componentCount` | `number` | Components registered |
| `cssFileCount` | `number` | CSS files produced |
| `protocolSizeBytes` | `number` | Protocol binary size |
| `tokenCount` | `number` | CSS tokens discovered |
