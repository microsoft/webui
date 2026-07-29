<!-- Copyright (c) Microsoft Corporation. -->
<!-- Licensed under the MIT license. -->

### streaming (Progressive Streaming Hydration, Phase 1)

Demonstrates a priority-ordered hydration scenario built on the
Progressive Streaming Hydration Phase 1 boundary contract from
`DESIGN.md` ("Progressive Streaming Hydration — Phase 1"): a
**`message-composer`** must paint and become
interactive before `DOMContentLoaded` while the response is still open; a
**`weather-panel`** shows its own skeleton and resolves itself off the
stream; a three-batch **feed** streams in afterward, each
batch's `feed-item` islands hydrating independently as their own
`<boundary>` commits.

```bash
# Install JS dependencies
pnpm install

# Build client JS + check the server compiles
pnpm build

# Run the custom Actix server (real webui::build + render_streaming)
pnpm start:server

# Run the Playwright suite
pnpm test
```

### Boundary order and pacing

| Sequence | Boundary      | Gap before the *next* flush        |
| -------- | ------------- | ---------------------------------- |
| 0        | weather shell | none — the composer must not wait   |
| 1        | composer      | jittered 500–1000 ms                |
| 2        | feed batch 1  | jittered 500–1000 ms                |
| 3        | feed batch 2  | jittered 500–1000 ms                |
| 4        | feed batch 3  | none — the response closes promptly |

The feed gaps are bounded by `--feed-delay-min-ms` (500) and
`--feed-delay-max-ms` (1000) and are re-rolled per request, so repeated loads
do not look mechanically identical:

```bash
cargo run -p streaming-example-server -- --feed-delay-min-ms 200 --feed-delay-max-ms 400
```

### Why weather fetches instead of streaming

Phase 1 delivers boundaries strictly in document order, so a header that has
already been parsed cannot be filled from later in the same response. The
weather boundary therefore ships a *complete* island immediately — a
`weather-panel` in its `loading` state — and the component resolves its own
forecast from `/api/weather` (deliberately slower than a feed gap) after it
hydrates. That is the Phase 1 answer to slow backend data: never make the
critical island wait, and never leave a skeleton that resolves to nothing.

This is also why it is a boundary at all rather than plain markup: the panel
has to hydrate before it can fetch, and boundary 0 gets it interactive without
delaying the composer, which commits in the same flush window.

Filling an earlier, already-closed region from later in the same response is
the deferred-boundary placement work described in `DESIGN.md`; the
`resolveBoundaryRange()` seam in `streaming.ts` exists for exactly that.

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
itself, and it is why this example demonstrates the boundary-carried loader
but not a preload strategy. The shared runtime chunk is a static import of
`index.js`, so the preload scanner cannot see it — the browser only discovers
it after downloading and parsing `index.js`, and that waterfall costs a round
trip.

Measured over a throttled link (100 ms RTT, 1.6 Mbps, deterministic pacing,
12 cold contexts, median composer time-to-interactive; per-run spreads were
~25 ms and did not overlap):

| Variant                        | Critical JS | Composer interactive |
| ------------------------------ | ----------- | -------------------- |
| Island bundled into `index.js` | 46,560 B    | 1074 ms              |
| Island split, no preload hint  | 45,912 B    | 1061 ms              |
| Island split, hint smallest-first | 45,912 B | 1076 ms              |
| Island split, hint largest-first  | 45,912 B | **956 ms**           |

Two things are worth taking away. Splitting alone is a wash: 648 bytes cannot
pay for a round trip, and it would be a straight loss for an island much
smaller than its share of the transfer. A `<link rel="modulepreload">` for the
shared chunk is what makes it pay, and its *order* matters more than the split
— preloads are issued in document order and share the connection, so listing
the 284-byte chunk ahead of the 35 KiB one delays the long pole and gives back
the entire win.

The two hinted rows were measured with a throwaway build manifest and are
recorded here as **motivation, not instruction**. Bundlers content-hash chunk
filenames, so reproducing them means an application-owned bundler plugin, a
manifest on disk, and a server that templates raw HTML into `<head>` — far too
much ceremony to recommend, and the wrong layer besides. WebUI already knows
each boundary's component closure and which build output defines each
component, so it should emit these hints itself through the head-injection path
it already uses for CSS. Until it does, this example ships the split without
the hint: the architectural win (island code loads with its boundary) without
pretending the time-to-interactive win is available to you.

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

The server (`server/src/main.rs`) calls the real, opt-in
`WebUIHandler::render_streaming` over a real `StreamingWriter`. Because a
fast in-process render would otherwise commit every boundary in the same
scheduler tick, `server/src/paced_writer.rs` wraps the writer in a narrowly
scoped, example-only `CheckpointPacedWriter`: it sleeps *after* delegating
each flush to the real writer, so backpressure and disconnect propagation are
unaffected — no envelope or chunk is ever manufactured or split by hand. The
delay is chosen per flush index by a caller-supplied schedule, which is what
lets the weather-to-composer gap stay at zero while the feed gaps are paced.

Three feed batches are three explicit `<boundary>` groups — Phase 1 does not
implement an open-ended `<webui-stream>` directive. The feed's `<section>`
container is never itself hydrated: each `feed-item` carries its own state in
its own attributes, so one batch's items can never read or mutate another
batch's state.
