# Desktop application IPC (internal contract)

This specification records the cross-language desktop **application IPC**
boundary. It complements the [desktop architecture](../DESIGN.md#desktop-boundary)
and [public usage guide](../docs/guide/concepts/desktop.md); it is not an
application API reference. The Rust host and its packaged renderer runtime are
built together. Application messages use generated proto3 codecs, while the
framework envelope is a separate, version-3 fixed-layout binary format. Exact
byte offsets, enum values, limits, and error codes are owned by the linked
sources, not duplicated here.

## Ownership and trust boundary

- Application IPC is opt-in (`application-ipc` plus an explicitly configured
  schema). An unconfigured registry disables admission; it does not start IPC
  workers or a timer. IPC is distinct from ordinary custom-protocol resources,
  route/API `fetch`, native lifecycle events, and window-control messages.
- One `DesktopFrame` owns mutable IPC state. Immutable generated schema and
  handler definitions can be shared; `IpcWindow`, `IpcSession`, and native
  `IpcBridge` handles are weak. A session is tied to its admitted document
  generation, never silently retargeted to the next document. A custom backend
  must advertise and implement application IPC before running an IPC-enabled
  frame.
- The authority to admit a renderer comes from native tracking of a **committed
  top-level document at the fixed application origin**, not a URL or navigation
  ID asserted in a renderer message. Native adapters own navigation identities,
  commit/probe sequencing, origin checks, and stale-completion rejection. They
  must not execute application handlers in native UI/protocol callbacks.
  WebKitGTK's control callback does not provide a trustworthy sending-frame
  identity: authorization therefore relies on possession of the
  native-authorized document capability and the browser's origin boundary,
  **not** a universal claim that every control callback is main-frame-only.
  Same-origin delegation is inside the trusted application boundary; do not
  treat untrusted same-origin content as isolated by this IPC design.

## Document proof and admission

1. The document-start bootstrap creates a per-document cryptographic nonce.
   After native commit, the adapter checks the app origin, probes that nonce in
   the current main document, and guards the asynchronous result against a
   changed navigation. `begin_document` records the trusted navigation and
   nonce with a new OS-random challenge. Activation evaluates in the current
   document; the wrapper checks the nonce **before** invoking the bootstrap,
   which checks it again. Neither challenge nor session token belongs in
   persistent scripts, URLs, logs, or availability controls.
2. The renderer's bounded native `hello` carries only wire version, generated
   contract name, contract major, and normalized schema hash, together with
   the echoed navigation/nonce/challenge proof and control correlation ID.
   Native/Rust admission compares it with the live activation, including both
   secret byte sequences, and requires the exact generated schema identity
   and current envelope version. Invalid, stale, or cross-document proofs do
   not consume a newer activation. No v2/legacy wire fallback or schema
   negotiation exists. The bootstrap property's historical `V2` suffix does
   **not** name the current wire format.
3. Exactly one successful admission consumes a document's activation and
   returns a new generation, random session token, and validated limits.
   An unused proof can remain available for a lazy first connection until the
   document retires. The bridge's bounded handshake deadline includes worker
   queue time and is rechecked before session delivery; the renderer also
   bounds its pending hello. A failed or undeliverable result may retire only
   its authenticated generation/token, never a replacement document.

Full-document navigation revokes prior proof, session, pending work, and
subscriptions; same-document routing does not. Trusted `pagehide` retires the
renderer connection. A cache-eligible document prepares a fresh bootstrap
nonce before freezing, but restoration still needs the native
commit/probe/activation path and an **explicit application reconnect**. Neither
old credentials nor old calls are replayed. Authenticated disconnect requires
both the current generation and token; generation alone is correlation, not
authorization. Frame shutdown is terminal.

## Application schema and method authority

The build generator emits Rust/TypeScript payload codecs and validation
metadata, receiver/kind/ID descriptors, a normalized descriptor hash, and a
compatibility lock. Methods have explicit unique IDs above the framework's
reserved range (greater than 1023); removed IDs are retired. Schema changes
require matching generated host and renderer bindings. A protobuf application
payload is opaque to the framework envelope; generated value and bounded,
iterative wire validators guard types, lengths, nesting, collections, and
oneofs before decoding. Protobuf byte ordering is not an identity proof; the
handshake uses the normalized schema hash.

Registering a Rust handler or declaring a generated method does **not** grant
renderer authority. `IpcOptions` denies both directions by default.
`for_schema` deliberately grants the declared host- and renderer-receiver
IDs; callers may instead narrow the respective allowlists. The host checks
the declared method ID, receiver direction, RPC/notification kind, and grant
at dispatch. Development-only methods additionally require the explicit
development setting **and** a source-backed host; packaged hosts deny them.
Absent receivers fail explicitly rather than inventing an implementation.

## Data transport and completion semantics

The native control channel carries bounded JSON admission, availability, and
closure metadata, never application DTOs. After admission, binary frames
`POST /_webui/ipc` with the session token in `X-WebUI-Ipc-Session`;
`GET /_webui/ipc/outbound` drains the host's bounded queue. These routes use
the app origin, binary response bodies and `application/x-protobuf` content
type (including standalone encoded error responses); that MIME name does
**not** mean the framework envelope is protobuf. A POST `204` accepts ingress,
not handler completion. A `ready` control wakes the renderer to drain until
`204`, without idle polling. A `closed` control retires that document's
connection. Resource, control, and application-data paths remain distinct.

Both sides use the same strict v3 frame codec: a fixed-layout little-endian
header with version, generation, sender-local nonzero ID, kind, method ID and
timeout, followed by a tagged payload, error, or absent body. Payload bytes
remain application protobuf. Requests receive `RESULT`/`ERROR`; notifications
receive `ACCEPT`/`ERROR` for admission, **not** callback completion. Cancellation
uses `CANCEL`; sender IDs increase without reuse per direction. Reject malformed
layouts, unknown kinds, invalid kind/body combinations, wrong version or
generation, duplicate invocations, and invalid direction before invoking an
application handler. Unknown/late completions do not reexecute work. There
are no implicit application retries.

## Bounded work and failures

Frame size, control size, queue slots/bytes, pending calls, notification fanout,
callback tasks, worker tasks (including retired documents), admitted input and
retained bytes, schema depth, collection entries, and deadlines are bounded.
Reserve transport capacity before copying or growing native/browser buffers;
hold credits with queued output and active or retired work until that storage
or work actually releases them. Keep a separate bounded control reserve so
payload saturation does not starve cancellation and error/completion delivery.
Workers and timer supervision start lazily; SDK workers run payload validation,
decoding, and Rust handlers away from native UI completion drivers. These
limits cover SDK-owned resources, **not** arbitrary memory or blocking work in
application handlers.

Deadlines include local queue time and use monotonic time where the call runs.
Cancellation, timeout, navigation, dropped callers, and close settle local
waiters without promising to stop synchronous work already running or undo
side effects. Per-subscription notification delivery preserves receipt order;
disposing a subscription prevents queued callbacks from starting, not an
already-running callback from finishing.

Failures have stable `IpcErrorCode` categories and bounded plain-text
`WireError` fields (`code`, `message`, `help`, optional application code).
Credentials and foreign exception stacks must not cross as diagnostics:
renderer failures use fixed SDK text; Rust handlers may supply bounded
`IpcError` text, so host authors must not put secrets in it. Invalid input is
rejected rather than dispatched; an HTTP error response does **not** by itself
imply that the document/session has closed. Transport and navigation closure
are terminal for that connection; overload, invalid payload, unsupported
version, schema mismatch, permission denial, deadline, and handler errors
remain distinguishable.

## Sources

- Rust boundary, proof, policy, lifecycle, wire and payload validation:
  [`ipc/mod.rs`](../crates/webui-desktop/src/ipc/mod.rs),
  [`ipc/admission.rs`](../crates/webui-desktop/src/ipc/admission.rs),
  [`ipc/bridge.rs`](../crates/webui-desktop/src/ipc/bridge.rs),
  [`ipc/engine.rs`](../crates/webui-desktop/src/ipc/engine.rs),
  [`ipc/session.rs`](../crates/webui-desktop/src/ipc/session.rs),
  [`ipc/registry.rs`](../crates/webui-desktop/src/ipc/registry.rs),
  [`ipc/limits.rs`](../crates/webui-desktop/src/ipc/limits.rs),
  [`ipc/error.rs`](../crates/webui-desktop/src/ipc/error.rs),
  [`ipc/validation.rs`](../crates/webui-desktop/src/ipc/validation.rs),
  [`ipc/wire.rs`](../crates/webui-desktop/src/ipc/wire.rs).
- Renderer boundary and codec:
  [`bootstrap.ts`](../packages/webui-desktop/src/bootstrap.ts),
  [`transport.ts`](../packages/webui-desktop/src/transport.ts),
  [`connection.ts`](../packages/webui-desktop/src/connection.ts),
  [`limits.ts`](../packages/webui-desktop/src/limits.ts),
  [`budget.ts`](../packages/webui-desktop/src/budget.ts),
  [`queue.ts`](../packages/webui-desktop/src/queue.ts),
  [`framing.ts`](../packages/webui-desktop/src/framing.ts),
  [`envelope.ts`](../packages/webui-desktop/src/envelope.ts),
  [`validation.ts`](../packages/webui-desktop/src/validation.ts),
  [`types.ts`](../packages/webui-desktop/src/types.ts).
- Native identity/epoch and integration:
  [`document.rs`](../crates/webui-desktop/src/document.rs),
  [`native_ipc.rs`](../crates/webui-desktop/src/native_ipc.rs),
  [`frame.rs`](../crates/webui-desktop/src/frame.rs),
  [`windows/ipc.rs`](../crates/webui-desktop/src/windows/ipc.rs),
  [`macos/ipc.rs`](../crates/webui-desktop/src/macos/ipc.rs),
  [`linux/ipc.rs`](../crates/webui-desktop/src/linux/ipc.rs),
  [`linux/ipc_message.rs`](../crates/webui-desktop/src/linux/ipc_message.rs).
  Generator identity/ID rules: [`compiler.rs`](../crates/webui-desktop-build/src/compiler.rs),
  [`evolution.rs`](../crates/webui-desktop-build/src/evolution.rs).
- Executable boundary checks:
  [`ipc_contract.rs`](../crates/webui-desktop/tests/ipc_contract.rs),
  [`wire.rs` unit tests](../crates/webui-desktop/src/ipc/wire.rs),
  [`validation.test.ts`](../packages/webui-desktop/test/validation.test.ts),
  [`bootstrap.test.ts`](../packages/webui-desktop/test/bootstrap.test.ts),
  [`transport.test.ts`](../packages/webui-desktop/test/transport.test.ts),
  [`connection.test.ts`](../packages/webui-desktop/test/connection.test.ts),
  [`native-lifecycle.test.ts`](../packages/webui-desktop/test/native-lifecycle.test.ts).
