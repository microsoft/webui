# @microsoft/webui-desktop

Browser-only runtime for generated, bidirectional WebUI desktop IPC. It does not
import Web Components, the framework, the router, or the Node addon.

## Application use

Generate an application contract with `webui-desktop-build`, then bundle its
`ipc.ts` and generated WebUI payload codecs with this package:

```ts
import { createDesktopTransport } from '@microsoft/webui-desktop';
import { connectDesktop } from './generated/ipc.js';

const connection = await connectDesktop(createDesktopTransport(), {
  renderer: {
    async labelFor(item, context) {
      context.signal.throwIfAborted();
      return { text: `Item ${item.id}` };
    },
  },
  onError(error) {
    // Notification callback failures are local, structured errors.
    showFailure(error.code);
  },
});

const changed = connection.renderer.onChanged(async item => {
  await updateView(item);
});

await connection.host.save(item, { timeoutMs: 30_000, signal });
await connection.host.selected(item);

changed.close();
connection.close();
```

Importing generated `ipc.ts` bindings does not load the desktop runtime or
payload codecs. The first explicit `connectDesktop()` loads them; concurrent
calls share the load but create independent connections. Type-only imports
remain erased. No import connects, polls, retries, or grants native authority.
Module-loading errors reject `connectDesktop()` unchanged, before transport
activation; a failed module load is not retried by subsequent calls. Handshake
failures affect only their own connection.

Enable ESM code splitting and deploy all emitted chunks to retain the startup
transfer/parse savings. A single-file bundle still defers activation, but not
delivery of the runtime bytes. A direct value import from
`@microsoft/webui-desktop`, such as the transport import above, loads that
package normally. To defer it too, use
`const { createDesktopTransport } = await import('@microsoft/webui-desktop')`
inside the application's explicit connection action.

`save()` waits for the remote handler's RESULT, including `Promise<void>`
handlers. `selected()` waits only for bounded notification admission, not
subscriber completion. Notifications run in receipt order for each subscriber;
different subscribers can run concurrently. Subscription disposal skips queued
callbacks but cannot undo a callback that has already started. Notifications
share one decoded payload in a document; subscribers must not mutate it.

Use `bigint` for all 64-bit integers and `Uint8Array` for bytes. The generated
validators reject narrowing, invalid map keys and oneofs before encoding.
All protobuf maps use `Map<K,V>`, with string, boolean, number (32-bit), or
bigint (64-bit) keys matching the declared protobuf key type. Object dictionaries
and stringified numeric/boolean keys are rejected rather than coerced.
`__proto__`, `constructor`, and `prototype` are ordinary string Map data.
Generated field metadata bounds collection entries and nesting before decoding.

Timeouts include local queue time. Abort, navigation, transport failure and close
settle a call once. Cancellation is cooperative, not rollback. There are no
automatic retries, cross-document replies, or automatic reconnection.
`connection.closed` resolves with its
terminal `IpcError`. Without `onError`, callback failures surface through the
browser's `reportError` (or a thrown microtask), rather than disappearing.

### Back/forward cache restores

A trusted `pagehide` retires active connections and pending hellos with
`navigated`, revokes their credentials, and removes their subscriptions. Hash
changes and same-document History API navigation do not retire a connection.
No `unload` or `beforeunload` listener is installed.

When `pagehide.persisted` is true, the frozen bootstrap prepares a fresh
cryptographic nonce **before the page freezes**. Its readonly `documentNonce`
getter then exposes the new admission epoch, including when the native
post-commit probe runs before `pageshow`. Old connections, transports, proofs and
tokens remain terminal; delayed replies cannot settle a new epoch's hello.
Only bootstrap admission state is replaced, not application state. Retired
running callbacks retain their byte credits across explicit reconnects.

The application must explicitly create a **new transport and connection** on
restore and reinstall its subscriptions, for example:

```ts
window.addEventListener('pageshow', event => {
  if (!event.persisted) return;
  void connectDesktop(createDesktopTransport(), {
    renderer: rendererHandlers,
    onError: reportIpcError,
  }).then(connection => {
    installSubscriptions(connection);
    setCurrentConnection(connection);
  }).catch(reportIpcError);
});
```

The ordinary hello waits for fresh trusted-native activation if it is still in
progress. The native adapter must run its usual nonce-probe, navigation check and
guarded activation on a committed history restore. Neither lifecycle event
creates native authority or sends an automatic hello/RPC. There is no reload,
polling, retry or RPC replay. Non-cached pagehide remains terminal; a replacement
document receives a new bootstrap through normal document-start injection.

Navigation-related scheme cancellation must be ordered after the native
nonce-guarded closed control is observable in the outgoing document, or return
an authenticated `navigated` protocol error while that scheme task is live.
Once retirement is known, the transport preserves its terminal reason across
fetch/body aborts. An unexplained network failure remains `transport`; neither
an `AbortError` name nor a later pagehide retroactively changes settled calls.

## Generator and custom codec seams

The generated wrapper uses these exports:

- `MessageShape`, `FieldShape`, `validateValue`, `validateMessage`;
- `MessageCodec<T>`: `encode`, `decode`, `validate`, `validateBytes`;
- `ConnectionSchema`: `{ hello: Hello, methods: readonly MethodDefinition[] }`;
- `connect(transport, schema, { handlers?, onError? })`;
- `RuntimeConnection.call<T>(id, value, options?)`, `notify(id, value)`,
  `subscribe(id, callback)`, and atomic `register(handlers)`.

This is an erased implementation seam. Application authors should use generated
typed clients, not select a result type or method ID themselves.

For handwritten typed adapters, `createConnection(schema, transport, options)`
accepts `IpcSchema` (Hello fields plus typed descriptors). Its `Connection`
exposes `call`, `notify`, `handle` and `subscribe` using
`RpcDescriptor<Request, Response, Receiver>` and
`EventDescriptor<Payload, Receiver>`. Descriptor identity and endpoint direction
are checked at runtime. `options.setup(connection)` installs renderer handlers
before the transport handshake. Both APIs use the same connection implementation.

Custom codecs are trusted code. They must implement the same early value and
wire-boundary validation as generated codecs; IPC does not sandbox arbitrary
codec or application allocations.

## Native adapter seam

Inject **the built `dist/native-bootstrap.js` bytes** at document start, only in
eligible main frames, identically for source and packaged applications. The
artifact installs an immutable, frozen `window.__webuiDesktopIpcV2`:

```ts
interface NativeIpcBootstrap {
  readonly documentNonce: string;
  activate(proof: {
    navigation: string;
    documentNonce: string;
    challenge: string;
  }): boolean;
  hello(hello: Hello): Promise<SessionInfo>;
  subscribeControl(listener: (control: NativeControl) => void): Subscription;
  disconnect(generation: string, token: string): void;
}
```

The epoch nonce is 128 random bits from `crypto.getRandomValues`, represented as
32 lowercase hexadecimal characters. Installation is initially inactive. The
native adapter must read the nonce after a trusted main-document commit, guard
that asynchronous read against navigation, obtain a native challenge, then
evaluate a nonce-guarded wrapper in that same current document. The wrapper must
compare the current bootstrap's nonce **before calling `activate`**, so a
replacement page's fake method never receives a stale challenge. A mismatched
nonce or conflicting activation returns `false`. Never embed a challenge in a persistent
document-start script or activate in response to a JavaScript hello message.

The single hello waits for activation and has a five-second deadline covering
that wait. Connection, transport and bootstrap boundaries project only
`wireVersion`, `contractName`, `contractMajor` and `schemaHash`; local method
descriptors, codecs and extra own properties never cross native admission.
The posted hello has exactly nine fields, adding `navigation`, `documentNonce`,
`challenge`, `callId: "1"` and `kind: "hello"`. Native success and structured
error replies must echo the navigation, nonce, challenge and call ID with
`kind: "helloResult"`. Stale proof or call ID replies cannot settle
the handshake. SessionInfo strips proof fields.

Platform hooks:

- WKWebView and WebKitGTK:
  `window.webkit.messageHandlers.webuiDesktopIpc.postMessage(object)`, returning
  a Promise with a `helloResult` object. Native ready/closed pushes call
  `window.__webuiDesktopIpcReceiveV2(object)`.
- WebView2: `window.chrome.webview.postMessage(object)` and its `message` event
  for correlated replies and ready/closed controls.

Both globals are non-writable and non-configurable. Conflicting installation
fails. Only one transport can subscribe. `disconnect("0", "")` cancels a
not-yet-admitted local hello; that sentinel is never posted natively.
An admitted disconnect requires the matching generation and closure-held token,
and posts `{kind: "disconnect", generation, token}`. Wrong or stale credentials
cannot close the local bootstrap. The native bridge must atomically authenticate
both credentials, comparing the secret in constant time, before revoking that
specific session. Trusted-native close/navigation is a separate path.

The transport subscribes before hello, coalesces ready signals into a dirty bit,
and serializes POSTs. It drains GETs until 204, including a wake racing with that
final response, then does no polling and installs no idle timer. Binary frames
use `application/x-protobuf` and `X-WebUI-Ipc-Session`; tokens never enter URLs,
logs, or application events. Native adapters remain responsible for trusted
origin, committed-main-document navigation, and post-commit identity checks.
The authenticated principal is the committed main-document capability, not a
claim about the physical frame sending a native callback. Same-origin capability
delegation is inside the trust boundary; cross-origin documents without the
proof and session token cannot gain authority. GTK callbacks must not fabricate
main-frame identity. IPC-enabled GTK views disable page caching so history
navigation creates a fresh bootstrap, and cancelled revoked navigations stay
closed.

## Resource bounds

Negotiated revision-3 limits cap calls, notifications, output frames/bytes,
callback fanout, input capacities and retained binary bytes. Each serialized GET
has a separate small-control buffer, capped at `maxErrorTextBytesTotal + 128`
bytes (plus at most one same-sized transient growth copy), so CANCEL, ERROR and
ACCEPT remain receivable while application input credit is full. Application
frames must acquire input/retained credit before dispatch, including small
frames that initially fit that buffer. Larger stream buffers reserve credit
before allocation/growth. The same input
credit transfers to decoded handlers and remains held until actual completion,
even after cancellation or connection close. Shared notification payloads retain
credit until every admitted callback completes. No raw fanout copies are made.

Outgoing control reservations are independent of user-data credits. Saturated
data queues can still send bounded ERROR, ACCEPT and CANCEL frames. Oversized
results become errors instead of entering an unbounded response queue.
Temporary generated codec working space is conservatively reserved as
`4 * (maxFrameBytes + 16 * maxCollectionEntriesPerMessage + 256)`.
Consequently, tightly configured retained-byte limits can reject encoding before
the frame queue itself is full. These are safety bounds, not a claim to bound
all JavaScript heap overhead or trusted application allocations.

The collection-entry limit counts repeated elements and Map entries across the
message, matching the Rust guard. Singular fields, Map-entry key/value fields,
and packed-field headers do not consume an additional collection entry. Raw
varints are limited to ten bytes, with only bit zero permitted in byte ten,
before generated codecs can narrow an integer or length.

## Build, regeneration and native embedding

From this package directory:

```sh
pnpm build
node scripts/build.mjs
pnpm test
node scripts/generate-wire.mjs
```

The TypeScript build emits normal ESM and declarations under `dist/`.
`scripts/build.mjs` emits deterministic browser artifacts:

- `dist/native-bootstrap.js`: self-contained document-start IIFE;
- `dist/desktop-runtime.js`: self-contained ESM runtime for a reserved SDK asset.

The canonical envelope source is
`crates/webui-desktop/proto/webui_desktop.proto`. `generate-wire.mjs` updates the
fixed framework envelope helpers. Application payload codecs are emitted by
`webui-desktop-build` and import only this package's WebUI-owned reader and
writer helpers. No proto parser, compiler, Node API, base64 codec, third-party
protobuf runtime, or JSON application DTO code ships in the runtime.

Native embedding is an **explicit separate operation**, never a side effect of
the package build:

```sh
node scripts/sync-bootstrap.mjs --write ../../crates/webui-desktop/src/generated/ipc
node scripts/sync-bootstrap.mjs --check ../../crates/webui-desktop/src/generated/ipc
```

The SDK integrator owns that destination, checks in both artifacts, embeds them
from inside the published crate, and serves identical reserved assets for source
and bundle modes. `--check` compares bytes without rewriting them. The parent
workspace build should run `tsc && node scripts/build.mjs`; package manifest,
publish wiring and native embedding are integration-owned.

Tests use real `node:test`, generated WebUI payload codecs, bounded fake transports,
fake native channels, compile-time negative assertions, and the actual built
IIFE in an isolated JavaScript realm. They do not launch or automate a GUI.
