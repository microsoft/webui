# WebUI Native Node Module Handler

The `@microsoft/webui` npm package provides high-performance server-side rendering for Node.js / Bun / Deno. It uses a native addon with zero-copy Buffer access and streams rendered HTML fragments via callbacks for progressive rendering with minimal latency.

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
| `build(options)` | Build templates into a protocol. Returns `{ protocol, cssFiles, componentAssetFiles, stats }` |
| `render(protocol, state, options?)` | Render protocol with route matching. Returns the rendered HTML as a string |
| `renderStream(protocol, state, onChunk, options?)` | Render with streaming output. Each HTML fragment is passed to `onChunk` as it is produced |
| `inspect(protocol)` | Convert protocol to JSON for debugging |

### RenderOptions

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `entry` | `string` | `"index.html"` | Fragment ID to start rendering from |
| `requestPath` | `string` | `"/"` | URL path to match routes against |
| `plugin` | `string` | - | Handler plugin name (see [Plugins](/guide/concepts/plugins/)) |

`state` accepts either an object (auto-serialized) or a pre-stringified JSON string.

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

### BuildStats

| Field | Type | Description |
|-------|------|-------------|
| `durationMs` | `number` | Build time in milliseconds |
| `fragmentCount` | `number` | Total fragments |
| `componentCount` | `number` | Components registered |
| `cssFileCount` | `number` | CSS files produced |
| `protocolSizeBytes` | `number` | Protocol binary size |
| `tokenCount` | `number` | CSS tokens discovered |
