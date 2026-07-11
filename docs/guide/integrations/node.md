# WebUI Native Node Module Handler

The `@microsoft/webui` npm package provides high-performance server-side
rendering for Node.js, Bun, and Deno. It uses a native addon with direct
`Buffer` access, a buffered string path for normal rendering, and batched
callbacks for streaming responses.

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
import { renderStream } from '@microsoft/webui';

const protocol = readFileSync('./dist/protocol.bin');

const server = createServer((req, res) => {
  res.writeHead(200, { 'Content-Type': 'text/html' });
  renderStream(
    protocol,
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
import { render } from '@microsoft/webui';

const protocol = Bun.file('./dist/protocol.bin');
const protocolData = Buffer.from(await protocol.arrayBuffer());

Bun.serve({
  port: 3000,
  fetch(req) {
    const url = new URL(req.url);
    const html = render(protocolData, { title: 'Home' }, {
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
import { render } from '@microsoft/webui';

const protocol = Deno.readFileSync('./dist/protocol.bin');
const protocolData = Buffer.from(protocol);

Deno.serve({ port: 3000 }, (req) => {
  const url = new URL(req.url);
  const html = render(protocolData, { title: 'Home' }, {
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

| Function | Description |
|----------|-------------|
| `build(options)` | Build templates into a protocol. Returns `{ protocol, cssFiles, componentAssetFiles, warnings, stats }` |
| `render(protocol, state, options?)` | Render protocol with route matching through the native buffered-string path |
| `renderStream(protocol, state, onChunk, options?)` | Render with callbacks coalesced around a 16 KiB target before crossing into JavaScript |
| `inspect(protocol)` | Convert protocol to JSON for debugging |

### RenderOptions

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `entry` | `string` | `"index.html"` | Fragment ID to start rendering from |
| `requestPath` | `string` | `"/"` | URL path to match routes against |
| `plugin` | `string` | - | Handler plugin name (see [Plugins](/guide/concepts/plugins/)) |

`state` accepts either an object (auto-serialized) or a pre-stringified JSON string.

## Reusing the Prepared Protocol

Load `protocol.bin` once and keep the same `Buffer` object and plugin selection
for the lifetime of the server:

```js
const protocol = readFileSync('./dist/protocol.bin');

const server = createServer((req, res) => {
  const html = render(protocol, getState(req), {
    entry: 'index.html',
    requestPath: req.url,
    plugin: 'webui',
  });
  res.end(html);
});
```

The package stores plugin-bound native prepared protocols in a `WeakMap` keyed
by `Buffer` identity. Reusing the same buffer and plugin avoids protobuf
decoding and deterministic index construction on later full, partial, and
component-template requests. The package compares a retained byte snapshot and
invalidates the prepared entry if the buffer is mutated. Reading the file into
a new buffer per request defeats this optimization.

Use `render()` when the complete HTML string is needed. Use `renderStream()`
when the HTTP integration can make progress from callbacks; callbacks are
batched rather than invoked for every internal handler write.

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
| `cssFileNameTemplate` | `string` | `"[name].[ext]"` | Emitted asset filename template for Link-mode CSS and component assets. Tokens: `[name]`, `[hash]`, `[ext]` |
| `cssPublicBase` | `string` | - | Public URL/path prefix for Link-mode CSS hrefs |
| `legalComments` | `"inline" \| "none"` | `"inline"` | Preserve legal CSS comments inline, or strip all comments |
| `theme` | `string` | - | Design token theme JSON path or npm package name. Missing required CSS tokens fail the build (literal `var()` fallbacks are exempt) |

### BuildStats

| Field | Type | Description |
|-------|------|-------------|
| `durationMs` | `number` | Build time in milliseconds |
| `fragmentCount` | `number` | Total fragments |
| `componentCount` | `number` | Components registered |
| `cssFileCount` | `number` | CSS files produced |
| `protocolSizeBytes` | `number` | Protocol binary size |
| `tokenCount` | `number` | CSS tokens discovered |
