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
- Lazy delivery is another consumer of the neutral resource layer, not part
  of navigation or the element renderer. JSON and NDJSON are separately selected.
  The WebUI lazy entry only resolves hostless sections and their owner context.
- JSON partial consumers import resource registration but not the NDJSON reader.
  Selecting NDJSON changes content negotiation and makes the decoder available
  before the first response.
- Progressive HTML coordination is a separate opt-in graph. It shares template
  and style data contracts, not route-state framing or navigation cancellation.
- The delivery package has three task entries: routing, lazy regions, and shared
  resources. Their pure named exports do not install features; private
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
Lazy requests send `buildId` with nonempty inventory. The server rejects a
different build, and clients validate response identity before resource mutation.
Initial and partial payloads carry the identity. An old bitset is never
reinterpreted against a new artifact.

Template-resource and stylesheet-definition inventories are separate. Collection
can invalidate template availability without discarding document-owned styles.
They are acknowledgements of definitions/readiness, not of application-state
freshness, loaded route instances, or unloadable JavaScript modules.

Application state is never inferred from inventory. Route cache freshness,
component instance state, and resource retention have distinct owners.

## Partial records and cancellation

Both route and region consumers select the same `PartialTransport`. JSON
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
Owner-scoped authored ids are stable application addresses. Anonymous identities
are build-local.

Owner metadata retains only eager blocks and references to deferred roots.
A hostless resource bundles its private ordinary blocks. Nonnegative block
references select the owning component's eager table; negative references select
`-reference - 1` in the current fragment's private table; strings select another
resource. Shared named blocks can remain in the eager owner or in a shared
external section. State, events, refs, and repeat scope always remain on the
existing owner. No new hydration root, state store, or component lifecycle is
created for the deferred body.

The WebUI adapter uses metadata identity for parsed DOM caching and weak
block-to-fragment ownership. Active views retain their fragment; disconnected
owners retain only fragments needed for a later reconnect. Closing a body
releases its view ownership. Unhydrated SSR owners are pinned conservatively
until the renderer can take over that ownership.

A deferred response has `type: "webui-lazy"`, `version: 1`, `owner`,
`fragment`, `buildId`, both inventories, and `required` resource identities.
Template/function/style maps are omitted when unnecessary. `state` distinguishes
a complete or terminal response from resource-only metadata. Resource preload
does not call a state provider. Hosts can obtain metadata plus its small
acknowledgement without reparsing the metadata JSON.

NDJSON accepts a complete record or resources followed by one terminal state
record. Metadata-only EOF, duplicate terminal state, extra records, invalid
identity, and server error records fail explicitly. The terminal record may
include resources for newly active nested deferred regions. Resource preparation
and configured imports can overlap data loading; the caller receives state only
after all required work succeeds.

The loader keeps no instance state cache. One preload waiter can cancel without
cancelling shared resource work. An epoch change during loading causes one
bounded retry using fresh inventory and state; repeated collection is an
actionable error, not an incomplete view or document-navigation fallback.

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
