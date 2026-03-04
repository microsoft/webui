# WebUI Node.js Handler

The WebUI Node.js handler provides high-performance server-side rendering via a native addon built with [napi-rs](https://napi.rs). It compiles the Rust handler directly into a `.node` addon — no C ABI intermediary — and supports true streaming SSR by delivering rendered fragments through a callback.

## Installation

Build the native addon from the WebUI workspace:

```bash
cargo build -p webui-node           # debug
cargo build -p webui-node --release # release
```

This produces a native addon library:

| Platform | Library file |
|---|---|
| macOS | `target/release/libwebui_node.dylib` |
| Linux | `target/release/libwebui_node.so` |
| Windows | `target/release/webui_node.dll` |

## Basic Usage

```js
import { readFileSync } from 'fs';
import { createRequire } from 'module';

// Load the native addon
const require = createRequire(import.meta.url);
const { render } = require('./target/release/libwebui_node.dylib');

// Read pre-compiled protocol (from `webui build`)
const protocol = readFileSync('./dist/protocol.bin');
const state = JSON.stringify({
  title: "Hello World",
  items: ["Milk", "Eggs", "Bread"]
});

// Render, streaming each fragment to stdout
render(protocol, state, (chunk) => process.stdout.write(chunk));
```

## API Reference

### `render(protocolData, stateJson, onChunk, plugin?)`

Render a pre-compiled WebUI protocol with JSON state data. Each rendered HTML fragment is streamed to the `onChunk` callback as it is produced.

**Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `protocolData` | `Buffer` | Protobuf binary from `webui build` (zero-copy) |
| `stateJson` | `string` | JSON string with the render state |
| `onChunk` | `(chunk: string) => void` | Called with each rendered HTML fragment |
| `plugin` | `string \| undefined` | Optional plugin identifier (e.g., `"fast"`) |

**Throws** on invalid protocol data, malformed JSON, or render errors.

## Streaming to an HTTP Response

The callback-based API enables true streaming SSR with any Node.js HTTP framework:

### Express

```js
import express from 'express';
import { readFileSync } from 'fs';
import { createRequire } from 'module';

const require = createRequire(import.meta.url);
const { render } = require('./target/release/libwebui_node.dylib');
const protocol = readFileSync('./dist/protocol.bin');

const app = express();

app.get('/', (req, res) => {
  res.setHeader('Content-Type', 'text/html');
  const state = JSON.stringify({ title: "My App", items: [] });
  render(protocol, state, (chunk) => res.write(chunk));
  res.end();
});

app.listen(3000);
```

### Node.js HTTP

```js
import { createServer } from 'http';
import { readFileSync } from 'fs';
import { createRequire } from 'module';

const require = createRequire(import.meta.url);
const { render } = require('./target/release/libwebui_node.dylib');
const protocol = readFileSync('./dist/protocol.bin');

const server = createServer((req, res) => {
  if (req.url === '/') {
    res.writeHead(200, { 'Content-Type': 'text/html' });
    const state = JSON.stringify({ title: "Home" });
    render(protocol, state, (chunk) => res.write(chunk));
    res.end();
  }
});

server.listen(3000);
```

## Using Plugins

Pass a plugin name as the fourth argument to enable framework-specific rendering:

```js
// Render with FAST-HTML hydration markers
render(protocol, state, (chunk) => res.write(chunk), 'fast');
```

When `"fast"` is specified, the handler injects hydration comment markers that FAST-HTML's client-side runtime uses to locate and re-hydrate dynamic content. See [Plugins](/guide/concepts/plugins/) for details.

## Example

A complete working example is available at `examples/integration/node/`:

```bash
# 1. Build the native addon
cargo build -p webui-node

# 2. Build the hello-world protocol
cargo run -p webui-cli -- build examples/app/hello-world/src \
  --out examples/app/hello-world/dist

# 3. Run the example
cd examples/integration/node
node index.js

# With FAST plugin
node index.js ../../app/todo-fast/dist/protocol.bin \
  ../../app/todo-fast/data/state.json --plugin=fast
```

## Performance Notes

- The native addon uses **zero-copy Buffer** access — protocol data is read directly from the Node.js `Buffer` without copying into Rust.
- Each `onChunk` call crosses the Rust→JS boundary, so fragment-level streaming gives you progressive rendering with minimal latency.
- The addon is stateless per call — create a new `render` invocation per request. The protocol data can be loaded once and reused across all requests.
