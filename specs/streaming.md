# Progressive response contract

This is the internal contract shared by the Rust handler, host bindings, and
browser coordinator. See [DESIGN.md](../DESIGN.md#progressive-streaming-hydration)
for the architectural boundary, the
[boundary guide](../docs/guide/concepts/directives/boundary.md) for authoring,
and the [hydration guide](../docs/guide/concepts/hydration.md#progressive-streaming-hydration)
for browser integration. The [hydration contract](hydration.md) covers how
complete streamed ranges adopt their SSR DOM. Streaming HTML is opt-in;
normal rendering and JSON/NDJSON navigation do not acquire its records
or retained state.

## One host-driven session

The compiler places declarations in entry or component records. Only a
declaration reached by the selected route and evaluated branches becomes a
response-local occurrence. Boundaries cannot nest or execute inside a repeat;
a repeat inside a boundary completes within that boundary. An occurrence has
a build-local declaration ID and a gapless response-local instance ID. If a
component declaration can have multiple live occurrences, the author supplies
keys unique within that declaration.

`start(initialState)` writes the shell up to the first occurrence or through
terminal when none exists. `resume(pendingInstanceId, state, mode)` writes and
flushes **only** the pending occurrence and its checkpoint. `advance()`
writes the following parent bytes until another occurrence or terminal.
`update(committedUpdatableId, state)` writes a markerless state record for
that occurrence; it never rerenders its HTML. `Final` releases roots after
hydration, while `Updatable` retains them until terminal. A session has one
driver. Wrong step or wrong occurrence fails before writing that step;
updates require object-valued state. Once a transport write fails,
previously committed bytes cannot be rolled back or repackaged as a
successful response.

The continuation retains only the reachable parent-state projection and
lexical scope, not the entire original state on each resume. Resolution
within a resumed occurrence checks lexical locals, the supplied resume
state, then frozen parent state. Independent host bindings return writable
segments rather than requiring callers to own a Rust writer. A separate
versioned NDJSON API-proxy control stream drives this same session through
the CLI; its `start`/`resume`/`update` commands are **not** browser hydration
records. See the [CLI reference](../docs/guide/cli/index.md).

## Browser record wire

The handler emits one inert JSON payload script followed by a
`<webui-hydrate>` sentinel for each record. The only browser wire shape is
the unversioned four-element array
`[recordSequence, kind, target, payload]`:

| `kind` | Meaning | `target` |
| --- | --- | --- |
| `0` | Final checkpoint | Next boundary instance ID |
| `1` | Updatable checkpoint | Next boundary instance ID |
| `2` | State update | Previously committed updatable boundary ID |
| `3` | Completed generated component span | Span instance ID, a separate namespace |
| `4` | Terminal | `0` |

The sequence starts at zero and increases by one for **every** record,
including updates and span completions. Boundary IDs are also gapless, but
span IDs occupy their own namespace. Checkpoints attach to compiler-owned
range markers in the parsed DOM, not selectors or document-wide searches.
State updates and terminal have no range markers. A completed span can
activate eligible children before its enclosing component has finished,
without relaxing the usual parent hydration barrier for unrelated roots.

Each checkpoint carries the boundary-local state and the additive template,
inventory, style, and route information required after earlier records.
Already-published global metadata is not overwritten. A range payload may
use `stateRef` pointing to the **exact preceding range record** plus a
top-level `stateDelta`, instead of a full `state`, only when the prior
projection is a proven subset under the same server-state revision. The
browser resolves a new state object before activation; missing, forward,
stale, or malformed references fail the stream. Updates are independent
patches and never become the next range-state reference base. Boundary
state is not published as global application state.

At the structural `body_end`, after the response tail and any body
injection, the handler writes exactly one markerless empty terminal
`[nextSequence, 4, 0, {}]` and flushes. The browser releases response-local
references and retained updatable roots; it emits successful hydration
completion only after queued records and pending activation are accounted
for. Readers tolerate unknown terminal payload fields but do not accept
another record after terminal. `JSON.parse` rejects truncated payloads; the
tuple-shape gate rejects incompatible envelopes. Ordering, target, range,
and state errors fail closed. A document that finishes parsing before
terminal is truncated, not complete. Error cleanup is bounded and must not
turn an incomplete stream into a successful hydration event.

## Delivery and resource ownership

The host owns request pacing, backpressure, cancellation, concurrent-session
limits, and propagation of writer failures. A transport flush hands bytes
to the HTTP layer; it does not guarantee an intermediary delivered them.
The browser coordinator is an explicitly imported, single ordered commit
queue with bounded work and retained state, not a separate observer/timer per
record. Script nonces are request-specific and must agree with the host's CSP.
Ordinary non-streaming output remains independent of this protocol.

## Defining sources

- [Rust session and step ordering](../crates/webui-handler/src/streaming/session.rs),
  [record serializer](../crates/webui-handler/src/streaming/checkpoint.rs),
  and [writer/backpressure adapter](../crates/webui/src/streaming.rs).
- [Browser tuple and kind definitions](../packages/webui-framework/src/streaming-protocol.ts),
  [coordinator validation](../packages/webui-framework/src/streaming-coordinator.ts),
  and [range-state references](../packages/webui-framework/src/streaming-record-state.ts).
- [Browser pipeline tests](../packages/webui-framework/src/streaming-pipeline.test.ts)
  and [server streaming tests](../crates/webui-handler/src/streaming/session.rs)
  are executable compatibility checks; exact limits and error codes belong
  with these sources rather than this overview.
