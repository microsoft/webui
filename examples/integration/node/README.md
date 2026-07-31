# WebUI Node.js Integration Example

Minimal example showing how to use the `@microsoft/webui` npm package to build
templates and render HTML with state data — all from Node.js.

## Prerequisites

1. Build the native addon:

```bash
cargo build -p microsoft-webui-node
```

2. Build the `@microsoft/webui` package:

```bash
pnpm --filter @microsoft/webui build
```

3. Install workspace dependencies:

```bash
pnpm install
```

## Usage

Build the hello-world app and render it with state data:

```bash
node index.js
```

Or render a pre-built protocol with custom state:

```bash
node index.js ../../app/hello-world/dist/protocol.bin ../../app/hello-world/data/state.json
```

This uses the `@microsoft/webui` package API (`build()` and `Protocol`) which
automatically resolves the native addon from the workspace build output.
`Protocol.render()` and `Protocol.renderStream()` reuse the decoded protocol.

## In-process progressive streaming

`streaming-server.js` is a second, self-contained example: an ordinary
`node:http` server that renders a **progressive streaming response** in-process,
with no sidecar and no `webui serve`.

```bash
pnpm --filter @microsoft/webui --filter @microsoft/webui-framework build
node streaming-server.js
# http://127.0.0.1:3040
```

Flags: `--port`, `--batch-delay-ms`, `--job-delay-ms`.

### What it demonstrates

`protocol.streamResponse()` opens a session whose methods **return** the bytes
they produced rather than writing them anywhere. Your server keeps the socket:

```js
const session = protocol.streamResponse({ entry: 'index.html', requestPath: '/' });
const status = session.boundary('job-status');   // resolve names once

res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8' });
await write(res, session.writeShell(baseState));
await write(res, session.writeBoundary(status, statusState, 'updatable'));
// ... later, on the same response:
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

Because the host owns every write, Node's backpressure contract is preserved
exactly, and the same shape works behind Express, Fastify, Hapi, or a raw
socket.

The page renders a status island and three log batches. Watch the terminal
client to see the chunk timing:

```
status 200 at   7 ms
chunk 1  627B at    8 ms   shell (<head> ships before any work finishes)
chunk 2  670B at    8 ms   job-status boundary, committed as updatable
chunk 3  875B at  418 ms   log batch 1
chunk 4  687B at  823 ms   log batch 2
chunk 5  156B at  918 ms   update -> job-status, on this same response
chunk 6  626B at 1229 ms   log batch 3
chunk 7  128B at 1230 ms   terminal record
```

Chunk 5 is the interesting one: the slow job finishes *after* its boundary was
already committed and hydrated, so the server patches it with a 156-byte state
record instead of forcing a client-side fetch or replacing DOM.

### Scope

This example covers the **response** half of streaming: chunking, ordering,
updates, and backpressure. Its components are scriptless, so hydration comes
from the framework's streaming entry loaded straight from
`@microsoft/webui-framework`, with no bundler step.

For the full picture — interactive islands, boundary-carried module loading,
`modulepreload` scheduling, and the measured performance story — see
[`examples/app/streaming`](../../app/streaming/README.md).
