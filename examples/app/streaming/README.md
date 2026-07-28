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
