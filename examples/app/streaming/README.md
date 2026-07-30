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
`<boundary>` commits.

```bash
# Install JS dependencies
pnpm install

# Build client JS + check the server compiles
pnpm build

# Run the custom Actix server (real webui::build + stream_response)
pnpm start:server

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

The feed gaps are bounded by `--feed-delay-min-ms` (500) and
`--feed-delay-max-ms` (1000) and are re-rolled per request, so repeated loads
do not look mechanically identical. The forecast resolves independently, so its
state record can appear between any two feed checkpoints:

```bash
cargo run -p streaming-example-server -- --feed-delay-min-ms 200 --feed-delay-max-ms 400
```

### Why weather uses a state record

Boundary HTML is delivered strictly in document order, so the weather boundary
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
there. `server/src/app.rs` passes `dist/webui-projection.json` into the build
(it already did, for state projection), and WebUI emits the hints itself:

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

The server accepts `--css style|module|link`. It defaults to `style` because a
critical boundary should paint styled with zero extra round trips, and it
serves the compiler-generated stylesheets in memory so `link` and `module`
work too:

```bash
cargo run -p streaming-example-server -- --css module
```

Measured here (four feed items, cold context, 100 ms RTT): `style` is 12,228 B
with the composer styled at 147 ms, `module` is 10,061 B at 158 ms, and `link`
is 8,368 B at 268 ms. Time to interactive is ~616 ms in all three — it is gated
by the application bundle, not by CSS. See
`docs/guide/concepts/directives/boundary.md` for the full trade-off.

### How it stays deterministic

The server (`server/src/main.rs`) creates the real, opt-in
`WebUIHandler::stream_response` session over a real `StreamingWriter`. One
admitted blocking worker owns both objects for the response lifetime. A bounded
async command channel applies backpressure to ready backend work, while
`server/src/pacing.rs` races weather readiness against each feed delay and sends
commands in completion order. No envelope or transport chunk is manufactured or
split by the example.

The server is split so the streaming call is the first thing you read:
`main.rs` is the routes and `render_page`, `pacing.rs` is the demo-only
timing, `app.rs` is the protocol build and sample data, and `assets.rs` is
the cache-header policy for `dist/`.

Three feed batches are three explicit `<boundary>` groups — WebUI does not
implement an open-ended `<webui-stream>` directive. The feed's `<section>`
container is never itself hydrated: each `feed-item` carries its own state in
its own attributes, so one batch's items can never read or mutate another
batch's state.
