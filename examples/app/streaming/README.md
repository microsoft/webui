<!-- Copyright (c) Microsoft Corporation. -->
<!-- Licensed under the MIT license. -->

### streaming (Progressive Streaming Hydration, Phase 1)

Demonstrates a priority-ordered hydration scenario built on the
Progressive Streaming Hydration Phase 1 boundary contract from
`DESIGN.md` ("Progressive Streaming Hydration — Phase 1"): a
**`message-composer`** must paint and become
interactive before `DOMContentLoaded` while the response is still open; a
**weather** header stays a skeleton (Phase 1 does not block the composer on
expensive backend work); a three-batch **feed** streams in afterward, each
batch's `feed-item` islands hydrating independently as their own
`<webui-boundary>` commits.

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
each of the first few flushes to the real writer, so backpressure and
disconnect propagation are unaffected — no envelope or chunk is ever
manufactured or split by hand.

Three feed batches are three explicit `<webui-boundary>` groups (sequences
1–3, after the composer's sequence 0) — Phase 1 does not implement an
open-ended `<webui-stream>` directive. The feed's `<section>` container is
never itself hydrated: each `feed-item` carries its own state in its own
attributes, so one batch's items can never read or mutate another batch's
state.
