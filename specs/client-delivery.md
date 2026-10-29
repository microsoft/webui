# Client delivery and capability contract

This reference separates shared resource publication from navigation policy and
progressive HTML hydration. Public usage lives in the interactivity/routing
guides; `DESIGN.md` owns the high-level browser boundaries.

## Import boundaries

- The framework API entry has no installation side effects. Its explicit
  runtime installer owns the registry bridge, parser-readiness tracking, and
  scriptless hosts. Component classes still own mounting and authored behavior.
  Stylesheet installation is a separate adapter; required but uninstalled style
  support fails before an unstyled component mount.
- The template-data entry installs no rendering engine. Registration can notify
  an already selected renderer, but does not import one.
- One router owns URL/history and authoritative-chain reconciliation.
  Its constructor takes no configuration. `start()` selects already-configured
  policies once; action/preload listeners and selected transports are ready
  before it returns. Reconfiguration while active is rejected. No singleton,
  constructor/start duplication, or hidden optional dynamic import remains.
- Lazy regions are not a delivery consumer. The framework's optional `lazy.js`
  runtime fetches static template packs and registers them through the renderer
  bridge; it imports no router, transport, or NDJSON reader. The rendering core
  contains no lazy code beyond resolving string block references through an
  installable resolver; `lazy.js` extends the element itself, so pages without
  lazy regions ship nothing for them.
- JSON partial consumers import resource registration but not the NDJSON reader.
  Selecting NDJSON changes content negotiation and makes the decoder available
  before the first response.
- Progressive HTML coordination is a separate opt-in graph. It shares template
  and style data contracts, not route-state framing or navigation cancellation.
- The delivery package has two task entries: routing and shared resources.
  Their pure named exports do not install features; private
  implementation-file layout is not the public API.

The framework's side-effect allowlist retains every explicit installer,
including streaming failure and resume-hook modules, and excludes pure API modules.
Source-test fixtures containing authored definitions remain side-effectful.
Public-import graph tests use package manifests without source-path aliases;
mixing source and installed runtime aliases can duplicate the renderer.

## Resource readiness and inventory

Publish trusted template/style resources using the renderer's registration and
readiness bridge. A response inventory is acknowledged only after that
registration succeeds. CSS readiness retains the selected strategy's existing
native-link and cascade semantics; warming bytes is not permission to apply
unverified CSS.

Within one inventory epoch, concurrently completing responses merge their
availability bitsets instead of replacing one another's bits. Explicit template
collection resets inventory and advances a document-local epoch. A request
captures the epoch before starting; after collection its resource data may still
be accepted, but its old inventory acknowledgement cannot resurrect evicted bits.

This local epoch is distinct from the compiler's deterministic artifact identity.
Initial and partial payloads carry the identity, and clients validate response
identity before resource mutation. An old bitset is never reinterpreted against
a new artifact. Lazy-region packs carry no identity: their URLs are
content-addressed and referenced only by their own build's metadata.

Template-resource and stylesheet-definition inventories are separate. Collection
can invalidate template availability without discarding document-owned styles.
They are acknowledgements of definitions/readiness, not of application-state
freshness, loaded route instances, or unloadable JavaScript modules.

Application state is never inferred from inventory. Route cache freshness,
component instance state, and resource retention have distinct owners.

## Partial records and cancellation

Route consumers select a `PartialTransport`. JSON
returns one decoded value; optional NDJSON returns the initial value plus an
owned, bounded record tail. The transport does not inspect route components,
apply state, or choose navigation fallback. A common request lifetime propagates
external cancellation and bounds the complete operation; disposing it removes
its timer and external abort listener. A failed consumer aborts the request
before disposal so rejected HTTP responses or transports cannot leave unread
bodies active.

Early intent preparation requests bounded JSON without importing a decoder.
The same preload policy is adopted at router startup, replacing the early
listener with the selected active policy. Destruction releases both stages.
After startup the router uses its explicitly selected transport.

The independent `NdjsonReader` retains one transport chunk plus a reusable
record buffer. Its default limit is 256 KiB of UTF-8 record bytes, excluding LF.
Limits apply to each record, not to the sum of coalesced records in one network
read. Split UTF-8 and an unterminated final line are supported. Malformed JSON,
invalid UTF-8, oversize records, and concurrent reader use fail explicitly.

The reader never validates or applies route state. It exposes a synchronous
buffered-record read so a route adapter can fold a state record delivered with
the first resource record into commit data. It does not allocate an array of
every line in the transport chunk.

The route adapter expects chain/resources first and positional `states` later.
It publishes resource readiness before returning metadata to the router, and
starts the deferred tail only after commit. If state arrived with metadata, it
is applied by that one commit, not replayed again afterward. Authored route
loaders preserve their precedence over deferred server state.

A malformed, server-error, or missing state tail cannot mark a cache entry
complete. Failures after commit are reported without claiming completion.
Cancellation releases buffered data and the underlying reader. The request
deadline covers both metadata and the deferred tail, including an unread tail
held for a pending commit.

Resource work can be shared; navigation generations and request cancellation
are consumer-owned. Cancelling one reader does not cancel a shared stylesheet
preparation promise.

## Lazy template bodies

`WebUIFragmentIf.lazy` extends the existing if instruction. `fragment_id`
still references its ordinary server body. Client payloads live in the
`client_fragments` table; they are not `ComponentData` entries or custom elements.
Region identities are compiler-owned and build-local
(`<owner>::lazy:@<ordinal>`); authors write no id.

Owner metadata retains only eager blocks and references to deferred roots.
A hostless resource bundles its private ordinary blocks. Nonnegative block
references select the owning component's eager table; negative references select
`-reference - 1` in the current fragment's private table; strings select another
resource. Shared named blocks can remain in the eager owner or in a shared
external section. State, events, refs, and repeat scope always remain on the
existing owner. No new hydration root, state store, or component lifecycle is
created for the deferred body.

### Compiled metadata

A lazy condition entry is `[condition, region, slot, trigger]`, where trigger
`1` is intent (the default for bare `lazy`), `2` idle, `3` load, and `4` demand.
The template JSON containing that entry, whether a component, a region, or a
shared block, carries `"lp": { region: url }` for the regions its conditions name.
URLs are the pack public base (default `_webui/packs/`, resolved against the
document base) plus `<owner>.<16 hex digits of SHA-256>.json`.

### Packs

A pack is `{ type: "webui-pack", version: 1, fragment, inventory,
styleInventory, required, bits, componentStyles?, templates?,
templateFunctions? }`. `templates` is the region's resource closure minus the
owner's eager closure; nested regions are separate packs. `required` lists
everything the region needs, including owner-provided resources. `bits` maps
each serialized template to its inventory bit. `componentStyles` is omitted
when the region adds no style resources or closures. Packs contain no state and
no build identity.

### Runtime

The rendering core resolves string block references through a resolver that
`lazy.js` installs and holds no other lazy logic. `lazy.js` extends the element:
when a closed region's condition becomes true and its template is not
registered, it loads the pack its container's `lp` names and re-enters the owner
once the pack is registered, which re-evaluates the condition. A region that
closed meanwhile stays unmounted, and a binding whose owner instance was
disposed cannot mount. Without the runtime, activation throws an error that
names `lazy.js`.

Activation and prefetch share one load per URL, with a 10 second deadline so a
stalled request fails and the next activation retries. A region whose pack is
still loading or installing is not ready even after its templates are
published: activation and prefetch join that load instead of mounting early.
An install that fails after publishing leaves the region not ready and keeps
only the published template identities, so template collection can still free
them; the next load registers those templates again so their readiness and
resource preparation rerun.
Installation validates
the type, version, and region; registers only templates the document does not
already hold (registered, still held by the template cache, which registers
them again, or still alive after release); runs their condition closures with
the document nonce and the compiler-owned Trusted Types policy; publishes them
through the registration event, waiting for its readiness; and then merges the
pack's inventory bits. Every template a pack names is registered at that point,
so its bits are safe to merge in any inventory epoch.

Triggers arm when the block containing a closed region renders or hydrates:
intent through one delegated document listener for pointer, focus, and touch
events whose composed path includes an element with the owner's tag; idle in an
idle callback after the window load event; load in the task after hydration.
Demand regions load only on activation or explicit prefetch. Idle and load
prefetch yield to `navigator.connection.saveData` and skip regions that were
already released. Prefetch failures are silent; activation surfaces errors and
retries.

The runtime remembers, weakly, the bindings that mounted each region it
registered. After instances are removed, a region that none of them still shows
is deleted from the registry at idle, its inventory bit is cleared, and a weak
reference is kept. Mounting it again revives the same metadata, and its bit,
while it is alive; lookups and trigger arming do not. Otherwise activation
reloads the pack, normally from the HTTP cache. Regions loaded only by prefetch
stay registered until they are shown and closed. Shared blocks and component
templates from packs stay registered; ordinary template collection governs them.
Collection in a new epoch leaves the runtime's earlier registrations to that
collection.

Setting a lazy condition is the whole activation API; prefetching is declared by
the trigger, never requested imperatively. `lazy.js` adds one optional method,
`$rendered()`, to elements and to their types through module augmentation, for
code that needs a body's DOM immediately. It flushes queued updates, then waits
for the owner's pending regions, including nested regions activated by their
mounts. It rejects with a pack failure and retries failed regions that remain
active when called again.

The WebUI adapter uses metadata identity for parsed DOM caching and weak
block-to-fragment ownership. Active views retain their fragment; disconnected
owners retain only fragments needed for a later reconnect. Closing a body
releases its view ownership. Unhydrated SSR owners are pinned conservatively
until the renderer can take over that ownership.

## Validation

Application-profile bundles use public package imports and test emitted static
and dynamic graphs, not just source filenames or parsed-but-eliminated inputs.
Helper-only imports must exclude renderer/navigation implementations and
perform no installation. JSON-only routing must exclude the NDJSON implementation,
and resource-only consumers must exclude route reconciliation and rendering.
Existing progressive-bootstrap size ceilings continue to apply to its initial
graph; they are not a whole-application budget.

Unit and real compiler/browser fixtures cover registration ordering, cache
collection, hidden dependencies, record framing, cancellation, and navigation
after collection. Memory probes report retained objects separately from raw
heap totals and do not require immediate garbage collection for correctness.
