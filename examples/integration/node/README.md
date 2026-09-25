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

res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8' });
let step = session.start(baseState);
await write(res, step.bytes);

const status = step.boundary; // runtime descriptor discovered by start()
step = session.resume(status.instanceId, statusState, 'updatable');
await write(res, step.bytes);
// The checkpoint is committed, but parent bytes have not advanced yet.
await write(res, session.update(status.instanceId, { jobState: 'succeeded' }));

while (!step.done) {
  const boundary = step.boundary;
  if (boundary) {
    step = session.resume(boundary.instanceId, await stateFor(boundary));
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

Because the host owns every write, Node's backpressure contract is preserved
exactly, and the same shape works behind Express, Fastify, Hapi, or a raw
socket.

The page renders a status island and three log batches. Watch the terminal
client to see the chunk timing:

```
status 200 at   7 ms
start              <head> ships and job-status is discovered
resume             commit job-status only, as updatable
update             patch job-status before advancing its parent
advance            write parent bytes and discover log batch 1
resume / advance   commit log batch 1, then discover log batch 2
resume / advance   commit log batch 2, then discover log batch 3
resume / advance   commit log batch 3, then write the tail and terminal
```

The update is the interesting one: the slow job finishes *after* its boundary was
already committed and hydrated, so the server patches it with a 156-byte state
record instead of forcing a client-side fetch or replacing DOM.

The host never resolves authored names before `start()`. Every runtime
descriptor comes from the preceding `StreamStep`, which also covers boundaries
discovered through routes, conditions, or component templates. A
boundary-bearing subtree under `<for>` fails with `boundary-in-repeat`; a whole
`<for>` may sit inside one boundary.

The state machine has no ambiguous unfinished step: a descriptor means
`resume`, neither a descriptor nor `done` means `advance`, and `done` means
complete. Boundary-only `resume` makes each checkpoint independently writable.
`advance` carries the following parent or tail bytes, so no sibling boundary is
needed.

### Scope

This example covers the **response** half of streaming: chunking, ordering,
updates, and backpressure. Its components are scriptless, so hydration comes
from the framework's streaming entry loaded straight from
`@microsoft/webui-framework`, with no bundler step.

For the full picture - interactive islands, boundary-carried module loading,
`modulepreload` scheduling, and the measured performance story - see
[`examples/app/streaming`](../../app/streaming/README.md).
