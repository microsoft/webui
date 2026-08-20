<!-- Copyright (c) Microsoft Corporation. -->
<!-- Licensed under the MIT license. -->

### streaming (Progressive Streaming Hydration)

Demonstrates a priority-ordered hydration scenario built on the
Progressive Streaming Hydration boundary contract from
`DESIGN.md` ("Progressive Streaming Hydration"): a
**`message-composer`** must paint and become
interactive before `DOMContentLoaded` while the response is still open; a
**`weather-panel`** shows its own skeleton and receives server state through the
same response; a three-batch **feed** streams in afterward, each
batch's `feed-item` islands hydrating independently as their own
`<boundary>` commits. The document entry contains only one
`<streaming-page>` host. Its component template owns all five boundaries, so
the composer becomes interactive before the parent page's final tail and span
completion arrive. Each declaration is static; no boundary executes from a
`<for>` body.

```bash
# Install JS dependencies
pnpm install

# Build client JS and the Node API
pnpm build

# Run the Node API, client watcher, and webui serve together
pnpm start

# Run the Playwright suite
pnpm test
```

The client build's `dist/webui-projection.json` is required. It keeps every
checkpoint's initial state boundary-local; the server fails with a build
instruction instead of silently falling back to full-state payloads.

### Boundary order and pacing

| Boundary ID | Boundary      | Delivery               |
| ----------- | ------------- | ---------------------- |
| 0           | weather shell | immediate, updatable   |
| 1           | composer      | immediate, final       |
| 2           | feed batch 1  | jittered 500-1000 ms   |
| 3           | feed batch 2  | jittered 500-1000 ms   |
| 4           | feed batch 3  | jittered 500-1000 ms   |

The Node API feed gaps are bounded by `--feed-delay-min-ms` (500) and
`--feed-delay-max-ms` (1000) and are re-rolled per request, so repeated loads
do not look mechanically identical. The forecast resolves independently, so its
state record can appear between any two feed checkpoints. If it is still
pending at the last gap, the API sends the update before the resume control for
the final descriptor, because the CLI follows that boundary-only resume with
the final `advance`, which completes the session:

```bash
pnpm start:api -- --feed-delay-min-ms 200 --feed-delay-max-ms 400
```

For an HTML request, `webui serve` advertises
`Accept: application/x-webui-stream, application/json`. The API selects
streaming with the first media type and writes version-2 NDJSON controls:

```text
{"type":"start","version":2,"state":{...}}
{"type":"resume","boundary":{"owner":"streaming-page","name":"weather-shell"},"mode":"updatable","state":{}}
{"type":"resume","boundary":{"owner":"streaming-page","name":"composer-ready"},"state":{}}
{"type":"update","boundary":{"owner":"streaming-page","name":"weather-shell"},"state":{...}}
```

In this topology Node controls readiness and order, but does not render WebUI.
Each `resume` echoes the expected runtime descriptor's owner, name, and optional
typed key. The one-way channel has no acknowledgement carrying instance IDs, so
the CLI validates that selector against the current `StreamStep`, remembers the
committed descriptor-to-instance mapping for later updates, and rejects stale or
ambiguous targets. `declarationId` may be included as an extra validation field.
Core `resume` writes only that occurrence through its checkpoint. The CLI then
calls core `advance` to write following parent bytes through the next
descriptor or terminal. The backend sends no `advance` control. After it sends
the resume for the final descriptor, it closes its NDJSON body; the CLI's final
`advance` writes the tail and terminal. A capacity-one command channel and
Node's `response.write()` / `drain` contract propagate backpressure across the
loopback bridge.

This split is why the component-local composer needs no synthetic sibling
boundary. Its resume step can flush the child checkpoint, and advance later
writes the unfinished parent tail and span completion.

**This is one of two supported topologies.** This example uses the
**API-proxy** topology, which is the right fit when you want the CLI to own
rendering, static assets, and the browser transport.

If your Node service already terminates HTTP and you want WebUI **in-process**,
call `protocol.streamResponse()` from `@microsoft/webui` instead: it returns one
`Buffer` per call, so your own server writes the bytes. See
[`examples/integration/node/streaming-server.js`](../../integration/node/README.md#in-process-progressive-streaming),
which needs no sidecar. C, C#, and WASM expose the same session.

Both topologies are thin adapters over the same Rust session, so boundary
ordering, projection, the wire format, and every diagnostic are identical.

### Why weather uses a state record

Runtime boundary HTML is delivered in discovery order, so the weather boundary
ships a complete `weather-panel` in its `loading` state immediately. The host
commits it as `BoundaryMode::Updatable`, starts forecast work concurrently, and
calls `StreamingResponse::update` when the forecast resolves. The typed state
record uses the same open response and the checkpoint's compiled projection.

The browser applies the patch through WebUI reactivity. It does not issue a
second request, replace DOM, rerun hydration, or call `hydratedCallback()` again.
If the island module is still loading, the coordinator merges the patch into
pending activation state and hydrates once with the newest values.

### Loading an island's code with the island

`weather-panel` is not imported by `src/index.ts`. Its `<script>` lives inside
its own boundary, so the browser discovers it when that chunk reaches the
parser and the critical entry never carries its bytes:

```html
<boundary name="weather-shell">
  <script type="module" async src="./weather-panel.js"></script>
  <weather-panel status="loading"></weather-panel>
</boundary>
```

Arriving late is safe by construction. The boundary commits before the class
exists, so the coordinator stashes that boundary's state on the root and parks
it on `customElements.whenDefined('weather-panel')`, keeping
`webui:hydration-complete` open until it activates. Nothing in the framework
changes for this; it is the same path any late definition takes.

Splitting an island out has one hazard that matters more than the split
itself. The shared runtime chunk is a static import of `index.js`, so the
preload scanner cannot see it — without help the browser would only discover
it after downloading and parsing `index.js`, and that waterfall costs a round
trip.

Measured over a throttled link (100 ms RTT, 1.6 Mbps, deterministic pacing,
12 cold contexts, median composer time-to-interactive):

| Variant                        | Critical JS | Composer interactive |
| ------------------------------ | ----------- | -------------------- |
| Island bundled into `index.js` | 46,560 B    | 1074 ms              |
| Island split, no preload hint  | 45,912 B    | 1061 ms              |
| Island split, hint smallest-first | 45,912 B | 1076 ms              |
| Island split, hint largest-first  | 45,912 B | **956 ms**           |

Treat those absolute figures as machine-specific. What reproduces is the
*relative* result, and only a same-sitting A/B shows it: comparing a fresh run
against a stored number from another day measures the machine, not the change.
Re-running the last two rows back to back against one binary, toggling only
whether the hints are emitted, gives 1144 ms without them and 1068 ms with
them — a 6.7% win whose per-run ranges barely touch (`[1085, 1230]` against
`[1041, 1103]`, so the hinted *worst* run still beats the unhinted median).

Two things are worth taking away. Splitting alone is a wash: 648 bytes cannot
pay for a round trip, and it would be a straight loss for an island much
smaller than its share of the transfer. A `<link rel="modulepreload">` for the
shared chunk is what makes it pay, and its *order* matters more than the split
— preloads are issued in document order and share the connection, so listing
the 284-byte chunk ahead of the 35 KiB one delays the long pole and gives back
the entire win.

This example now sits in the bottom row, and it does not do anything to get
there. `build-client.mjs` emits `dist/webui-projection.json`, `webui serve`
consumes it for state projection and preload discovery, and WebUI emits the
hints itself:

```html
<link rel="modulepreload" href="./chunk-WKHXE3QO.js"><!-- 35,827 B -->
<link rel="modulepreload" href="./chunk-NKNSLYVV.js"><!--    284 B -->
```

Note what is *absent*: `weather-panel.js`. Its loader is inside a boundary, so
the build treats it as deferred by definition and never hoists it onto the
critical path — preloading it would undo exactly what splitting bought. The
shared chunk it uses is still preloaded, because `index.js` also imports it
statically, which makes it critical regardless of who else wants it.

### Trying the CSS strategies

`webui serve` accepts `--css style|module|link`. This example's
`start:server` script selects `style` because a critical boundary should paint
with zero extra round trips. The CLI serves compiler-generated stylesheets in
memory, so `link` and `module` work too:

```bash
cargo run -p microsoft-webui-cli -- serve ./src \
  --plugin=webui --projection-manifest ./dist/webui-projection.json \
  --servedir ./dist --api-port 3030 --port 3020 --css module
```

Measured here (four feed items, cold context, 100 ms RTT): `style` is 12,228 B
with the composer styled at 147 ms, `module` is 10,061 B at 158 ms, and `link`
is 8,368 B at 268 ms. Time to interactive is ~616 ms in all three — it is gated
by the application bundle, not by CSS. See
`docs/guide/concepts/directives/boundary.md` for the full trade-off.

### How it stays deterministic

`server/src/pacing.ts` races weather readiness against each feed delay and
writes controls in completion order. `server/src/stream-protocol.ts` emits one
record per semantic write, honors Node HTTP backpressure, and closes the body
after the resume control for the final descriptor. The CLI then performs the
final advance. The API caps concurrent admitted streams before sending a 200
response.

Inside `webui serve`, one blocking worker owns the real
`WebUIHandler::stream_response` and `StreamingWriter` for the response
lifetime. The async API reader feeds it through a capacity-one command channel;
the existing capacity-four browser channel and 30-second flush timeout remain
in force. The example never manufactures browser envelopes or duplicates
protocol rendering in JavaScript.

Three feed batches are three explicit component-local `<boundary>` groups — WebUI does not
implement an open-ended `<webui-stream>` directive. The feed's `<section>`
container is never itself hydrated: each `feed-item` carries its own state in
its own attributes, so one batch's items can never read or mutate another
batch's state.
