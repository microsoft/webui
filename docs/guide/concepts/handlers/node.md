# WebUI Native Node Module Handler

The `@microsoft/webui` npm package provides high-performance server-side rendering for Node.js / Bun / Deno. It uses a native addon with zero-copy Buffer access and streams rendered HTML fragments via callbacks for progressive rendering with minimal latency.

## Installation

```bash
npm install @microsoft/webui
```

## Examples

::: code-group
```js [Node.js]
import { createServer } from 'node:http';
import { readFileSync } from 'node:fs';
import { renderStream } from '@microsoft/webui';

const protocol = readFileSync('./dist/protocol.bin');

const server = createServer((req, res) => {
  if (req.url === '/') {
    res.writeHead(200, { 'Content-Type': 'text/html' });
    renderStream(protocol, { title: "Home" }, (chunk) => res.write(chunk));
    res.end();
  }
});

server.listen(3000);
```

```ts [Bun]
import { renderStream } from '@microsoft/webui';

const protocol = Bun.file('./dist/protocol.bin');
const protocolData = Buffer.from(await protocol.arrayBuffer());

Bun.serve({
  port: 3000,
  fetch(req) {
    const chunks: string[] = [];
    renderStream(protocolData, { title: "Home" }, (chunk) => chunks.push(chunk));
    return new Response(chunks.join(''), {
      headers: { 'Content-Type': 'text/html' },
    });
  },
});
```

```ts [Deno]
import { renderStream } from '@microsoft/webui';

const protocol = Deno.readFileSync('./dist/protocol.bin');
const protocolData = Buffer.from(protocol);

Deno.serve({ port: 3000 }, (_req) => {
  const chunks: string[] = [];
  renderStream(protocolData, { title: "Home" }, (chunk) => chunks.push(chunk));
  return new Response(chunks.join(''), {
    headers: { 'Content-Type': 'text/html' },
  });
});
```
:::

## API Reference

| Function | Description |
|----------|-------------|
| `build(options)` | Build templates into a protocol. Returns `{ protocol, cssFiles, stats }` |
| `render(protocol, state)` | Render protocol to HTML string |
| `renderStream(protocol, state, onChunk)` | Stream rendered HTML fragments via callback |
| `inspect(protocol)` | Convert protocol to JSON for debugging |

### BuildOptions

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `appDir` | `string` | — | Path to app folder |
| `entry` | `string` | `"index.html"` | Entry file |
| `css` | `"link" \| "style"` | `"link"` | CSS delivery strategy |
| `plugin` | `string` | — | Parser plugin (e.g. `"fast"`) |
| `components` | `string[]` | — | External component sources |

### BuildStats

| Field | Type | Description |
|-------|------|-------------|
| `durationMs` | `number` | Build time in milliseconds |
| `fragmentCount` | `number` | Total fragments |
| `componentCount` | `number` | Components registered |
| `cssFileCount` | `number` | CSS files produced |
| `protocolSizeBytes` | `number` | Protocol binary size |
| `tokenCount` | `number` | CSS tokens discovered |

