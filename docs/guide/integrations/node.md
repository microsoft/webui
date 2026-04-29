# WebUI Native Node Module Handler

The `@microsoft/webui` npm package provides high-performance server-side rendering for Node.js / Bun / Deno. It uses a native addon with zero-copy Buffer access and streams rendered HTML fragments via callbacks for progressive rendering with minimal latency.

## Installation

```bash
npm install @microsoft/webui
```

## Examples

<webui-tabs>
<webui-tab slot="tab" active>Node.js</webui-tab>
<webui-tab slot="tab">Bun</webui-tab>
<webui-tab slot="tab">Deno</webui-tab>
<webui-tab-panel active>

```js
import { createServer } from 'node:http';
import { readFileSync } from 'node:fs';
import { render } from '@microsoft/webui';

const protocol = readFileSync('./dist/protocol.bin');

const server = createServer((req, res) => {
  res.writeHead(200, { 'Content-Type': 'text/html' });
  render(protocol, JSON.stringify({ title: "Home" }), 'index.html', req.url,
    (chunk) => res.write(chunk));
  res.end();
});

server.listen(3000);
```

</webui-tab-panel>
<webui-tab-panel>

```ts
import { render } from '@microsoft/webui';

const protocol = Bun.file('./dist/protocol.bin');
const protocolData = Buffer.from(await protocol.arrayBuffer());

Bun.serve({
  port: 3000,
  fetch(req) {
    const url = new URL(req.url);
    const chunks: string[] = [];
    render(protocolData, JSON.stringify({ title: "Home" }), 'index.html', url.pathname,
      (chunk: string) => chunks.push(chunk));
    return new Response(chunks.join(''), {
      headers: { 'Content-Type': 'text/html' },
    });
  },
});
```

</webui-tab-panel>
<webui-tab-panel>

```ts
import { render } from '@microsoft/webui';

const protocol = Deno.readFileSync('./dist/protocol.bin');
const protocolData = Buffer.from(protocol);

Deno.serve({ port: 3000 }, (req) => {
  const url = new URL(req.url);
  const chunks: string[] = [];
  render(protocolData, JSON.stringify({ title: "Home" }), 'index.html', url.pathname,
    (chunk: string) => chunks.push(chunk));
  return new Response(chunks.join(''), {
    headers: { 'Content-Type': 'text/html' },
  });
});
```

</webui-tab-panel>
</webui-tabs>

## API Reference

| Function | Description |
|----------|-------------|
| `build(options)` | Build templates into a protocol. Returns `{ protocol, cssFiles, stats }` |
| `render(protocol, state, entry, requestPath, onChunk, plugin?)` | Render protocol with route matching, streaming HTML fragments via callback |
| `inspect(protocol)` | Convert protocol to JSON for debugging |

### BuildOptions

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `appDir` | `string` | - | Path to app folder |
| `entry` | `string` | `"index.html"` | Entry file |
| `css` | `"link" \| "style" \| "module"` | `"link"` | CSS delivery strategy |
| `plugin` | `string` | - | Parser plugin (e.g. `"fast-v3"`; deprecated `"fast"`/`"fast-v2"` keep @microsoft/fast-element 2.x compatibility) |
| `components` | `string[]` | - | External component sources |

### BuildStats

| Field | Type | Description |
|-------|------|-------------|
| `durationMs` | `number` | Build time in milliseconds |
| `fragmentCount` | `number` | Total fragments |
| `componentCount` | `number` | Components registered |
| `cssFileCount` | `number` | CSS files produced |
| `protocolSizeBytes` | `number` | Protocol binary size |
| `tokenCount` | `number` | CSS tokens discovered |
