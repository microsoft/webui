# Client delivery and capability contract

This reference separates shared resource publication from navigation policy and
progressive HTML hydration. Public usage lives in the interactivity/routing
guides; `DESIGN.md` owns the high-level browser boundaries.

## Import boundaries

- The framework core owns compiled mounting/hydration and authored behavior.
  Stylesheet installation is a separate adapter; required but uninstalled style
  support fails before an unstyled component mount.
- The template-data entry installs no rendering engine. Registration can notify
  an already selected renderer, but does not import one.
- The lean router owns URL/history and authoritative-chain reconciliation.
  Its constructor accepts coarse capabilities for optional policies; it has no
  static or dynamic import of their implementations.
- Deferred delivery is another consumer of the neutral resource layer, not part
  of navigation or the element renderer. JSON and NDJSON are separately selected.
  The WebUI deferred entry only resolves hostless sections and their owner context.
- JSON partial consumers import resource registration but not the NDJSON reader.
  Selecting NDJSON changes content negotiation and makes the decoder available
  before the first response.
- Progressive HTML coordination is a separate opt-in graph. It shares template
  and style data contracts, not route-state framing or navigation cancellation.
- Convenience entries may compose capabilities. They are not evidence that an
  unselected feature is absent from a lean application bundle.

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
Deferred requests send `buildId` with nonempty inventory. The server rejects a
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

## Deferred template bodies

`WebUIFragmentIf.deferred` extends the existing if instruction. `fragment_id`
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

A deferred response has `type: "webui-deferred"`, `version: 1`, `owner`,
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

Application-profile bundles test the emitted static and dynamic graphs, not
just source filenames. JSON-only routing must exclude the NDJSON implementation,
and resource-only consumers must exclude route reconciliation and rendering.
Existing progressive-bootstrap size ceilings continue to apply to its initial
graph; they are not a whole-application budget.

Unit and real compiler/browser fixtures cover registration ordering, cache
collection, hidden dependencies, record framing, cancellation, and navigation
after collection. Memory probes report retained objects separately from raw
heap totals and do not require immediate garbage collection for correctness.
