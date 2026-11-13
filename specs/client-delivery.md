# Client delivery and capability contract

This reference separates shared resource publication from navigation policy and
progressive HTML hydration. Public usage lives in the interactivity/routing
guides; `DESIGN.md` owns the high-level browser boundaries.

## Import boundaries

- The framework API entry has no installation side effects. Its explicit
  runtime installer owns the registry bridge, parser-readiness tracking, and
  scriptless hosts. Component classes still own mounting and authored behavior.
  Normal `startRuntime()` also installs the combined stylesheet adapter before
  activating declarations. The adapter has an isolated composition module;
  the legacy `styles.js` entry calls that same installer and retains its helper
  exports. Repeated startup or compatibility imports reuse stable runtime and
  registration functions instead of allocating another adapter.
- The template-data entry installs no rendering engine. Registration can notify
  an already selected renderer, but does not import one.
- One router owns URL/history and authoritative-chain reconciliation.
  Its constructor takes no configuration. `start()` selects already-configured
  policies once; action/preload listeners and selected transports are ready
  before it returns. Reconfiguration while active is rejected. No singleton,
  constructor/start duplication, or hidden optional dynamic import remains.
- Lazy regions are not a delivery consumer. The framework's optional `lazy.js`
  runtime fetches static template packs and registers them through the renderer
  bridge; it imports no router, transport, or NDJSON reader. Region component
  scripts are application code: a compiler-generated default module registers
  dynamic imports, or the application supplies tag-keyed loaders with the same
  pure generated dependency module. The application's bundler splits and serves them. The
  rendering core contains no lazy code beyond resolving string block references
  through an installable resolver; `lazy.js` extends the element itself, so
  pages without lazy regions ship nothing for them.
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

SSR bootstrap, component assets, lazy packs, and router responses share the
framework's Document-owned style catalog and installation engine. Component
assets register prepared resources directly; route responses use the shared
registration bridge once normal bootstrap is active. Renderer-independent
consumers keep their wire validation and pre-bootstrap publication behavior;
they do not import a second DOM style engine.

Within one inventory epoch, concurrently completing responses merge their
availability bitsets instead of replacing one another's bits. Explicit template
collection resets inventory and advances a document-local epoch. A request
captures the epoch before starting; after collection its resource data may still
be accepted, but its old inventory acknowledgement cannot resurrect evicted bits.
`ensureLoaded()` retains its requested resident metadata only for the operation,
restores requested entries removed during an awaited fetch, and verifies the
complete requested set before resolving. Its in-flight map contains no completed
payloads; later collection can release those entries normally. FAST availability
is its document-owned declarative template, not a WebUI metadata registry entry.

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

An optional response policy selects Fetch redirect behavior and classifies
response metadata before HTTP status rejection, content-type fallback, or
decoding. Inspection owns no body; the transport remains its sole reader.
Inspection rejection aborts and releases the body without publishing resources,
changing route state, or choosing document fallback. Its async lifetime is
abortable and bounded by the same request deadline. Only the current navigation
generation reports errors to the application; speculation and superseded
requests cannot present recovery UI.

Early preparation uses the same policy object as router startup or is discarded
at adoption. Prepared responses retain original response metadata for inspection
at adoption, even though their bounded body has been rewrapped. Speculative
classification cannot silently consume an error response as a cached success.
Default behavior stays unchanged when the policy is absent.

Policy-selected Navigation API interception propagates failure instead of
fulfilling against stale state. It awaits the deferred task owned by that
specific partial, not a router-global last reader, before reporting success.
Post-commit stream failure cannot restore the preceding DOM. The fallback
history implementation uses the same application error notification without
adding a promise-returning navigate API.

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

Node's public complete partial output may be application-framed as route NDJSON:
preserve the generated metadata record and move the returned top-level state
into positional state records with the same chain order. The current Node
producer emits only top-level state, never per-entry chain state. That projection
is shared across the matched tree, not split by route level. Hosts choose which
level owns which state: broadcasting a leaf ViewModel can overwrite retained
parent properties, whereas null parent slots preserve their state. The public
Node browser fixture asserts this producer invariant and verifies a leaf-only
tail preserves a parent-local value even when the leaf VM has a colliding key.
This is a buffered producer, not an asynchronous route-state session. Preparing metadata with
placeholder state is not equivalent: state-selected lazy resources and projected
state must agree with the real render input. A genuinely metadata-first Node
producer would require a separately defined native ownership/projection boundary,
not reuse of progressive HTML sessions or application-generated compiler data.

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
`1` is explicit intent, `2` idle, `3` eager, and `4` demand (the default for bare
and empty `lazy`). Activation evaluates the full condition expression. Path
prefetch selects its dependency paths, not expression text.
The author-facing name `load` is rejected with a hint to use `eager`.
The template JSON containing that entry, whether a component, a region, or a
shared block, carries `"lp": { region: path }` for the regions its conditions name.
Paths are `_webui/packs/<owner>.<16 hex digits of SHA-256>.json`, relative to the
asset base.

In link builds the same template also carries
`"lc": { region: [stylesheet path, ...] }`, for example
`"lc":{"app-shell::lazy:@0":["settings-form.css","theme-picker.css"]}`.
A region's list holds the `href` of every link resource its pack registers in
`componentStyles`, the same base-relative paths the pack carries, including
bundled chunks, in the order the pack's `closures` (sorted by root) first name
them, without duplicates. Regions whose packs register no link resource are
left out, and `lc` is omitted when none remains. Style and module builds never
emit it: their packs carry CSS text. Keys and order are deterministic, like `lp`.
The build report lists the same paths as each pack's `stylesheets`.

The browser resolves pack and stylesheet paths against `window.__webui.assetBase`
when the document has one, else against the document base URL. The handler
sends the response's asset base (the request's `$webui.assetBase`, else the
protocol's build default) as `assetBase` in the `#webui-data` object, or in the
first checkpoint record of a streamed response, and omits it when there is none.
Document renders, buffered or streamed, also write it at the head-end boundary,
right after `<meta name="webui-nonce">`, as
`<meta name="webui-asset-base" content="...">`. The attribute value is
HTML-escaped like the nonce (`/` becomes `&#x2F;`), so the element's `content`
reads back the base, and application code can read it before the runtime
parses `#webui-data`, for example to set a bundler's public path. Neither is
written without a base. Module preload hints never take the base: they name
the application's own entry-script imports, which WebUI does not relocate.

FAST 2/3 Link templates carry a compiler-owned `data-webui-stylesheet` marker on
their injected root link. Stored hrefs remain deployment-relative. Buffered and
streamed SSR prefix only this marked href during payload emission; partial and
component-template strings remain relative, and delivery resolves the marked
link against the document's runtime asset base, or its response metadata before
runtime startup, before FAST consumes the inner template. A resource-only
consumer does not need router or renderer initialization. Invalid configured
bases fail rather than redirecting resource work to the document origin.
Authored links are not marked or relocated. The marker is a literal
compiled substitution point, not a request-time HTML parsing step.

### Packs

A pack is `{ type: "webui-pack", version: 1, fragment, inventory,
styleInventory, required, bits, componentStyles?, templates?,
templateFunctions? }`. `templates` is the region's resource closure minus the
owner's eager closure; nested regions are separate packs. `required` lists
everything the region needs, including owner-provided resources. `bits` maps
each serialized template to its inventory bit. `componentStyles` is omitted
when the region adds no style resources or closures. Packs follow the build's
CSS strategy and DOM fallback like route partials: link builds name stylesheet
paths, which the browser resolves against the asset base and which the
template holding the region's condition also lists in `lc`; style and module
builds carry CSS text. Packs contain no state, no build identity, and no
JavaScript.

### Region scripts

Custom elements a region introduces are application code, delivered by the
application's bundler. When a build has lazy regions it emits `lazy.webui.js`
into the generated-modules directory:

```javascript
import { registerGeneratedLazyScripts } from '@microsoft/webui-framework/lazy.js';
registerGeneratedLazyScripts(
  { "<region>": ["<tag>"] },
  { "<tag>": () => import("<specifier>") },
);
```

A region's tags are the scripted components among the templates its pack
serializes (the same closure), except those the entry page loads at startup:
components the entry fragment reaches without crossing a lazy region or a
`<route>`. Packs still carry those templates. Components reached only through
routes stay listed, because they load with route partials. A component's
specifier is its sibling `.ts` or `.js` module, relative to the generated
module, or an explicit component-module mapping. Regions left without tags are
omitted. Each tag has one default loader even when several regions need it.
The call still runs with empty dependencies, so applications can always
import this default installer instead of `lazy.js`.

The registration bridge is internal to the generated installer, not an
application loader API. There is no separate dependency companion or callback
adapter. Registration starts no loads and does not walk component
dependencies. A missing generated mapping rejects when its region needs that tag, through
the same declaration-readiness check as a callback that resolves too early.
Each registration retains the original generated dependency object and callback
table once, with region lookups pointing to that shared registration, not a
copied per-region callback map or per-region tuple.

Next to it the build writes `lazy.webui.d.ts`, which is always:

```typescript
// Generated by WebUI. Do not edit.
import '@microsoft/webui-framework/lazy.js';
```

TypeScript resolves an import of `./lazy.webui.js` to this declaration, which
brings in the runtime's module augmentation (`$prefetch()`)
without `allowJs`. Declaration files are exempt from `rootDir`, so the
generated directory may sit outside it. Both files are published together and
rewritten only when their content changes.
CLI, `build_to_disk`, and watched rebuilds publish the module and declaration
transactionally, including collision checks and rollback.

One build registers its entire compiled region/tag graph at startup, even when
page behavior modules are dynamically imported. The maps and callbacks have a
real startup cost, but neither route code nor closed lazy code is evaluated by
registration. Independently built MPA documents own independent graphs. Bare
lazy installs no owner-wide intent listeners. Explicit intent remains owner-wide;
idle applies to its declaration. Nested lazy boundaries keep independent packs.

The build reports every pack with its path, region, owner, trigger, raw size,
content hash, script specifiers, and stylesheet paths, in the build results
and, with brotli sizes, as `lazyPacks` in the metafile, so deployments can
upload, precache, and budget packs without reading them. A pack's
`stylesheets` is the `lc` list of its region; the build writes both from one
value, so they cannot differ. The field is always present: empty in style and
module builds, and for regions whose packs register no link resource, like
`scripts` for regions without scripts. Precaching a region needs its pack path,
its stylesheets, and the bundler's outputs for its scripts.

### Runtime

The rendering core resolves string block references through a resolver that
`lazy.js` installs and holds no other lazy logic. `lazy.js` extends the element:
when a closed region's condition becomes true and its template is not
registered, it loads the pack its container's `lp` names and re-enters the owner
once the pack is registered, which re-evaluates the condition. A region that
closed meanwhile stays unmounted, and a binding whose owner instance was
disposed cannot mount. Without the runtime, activation throws an error that
names the generated `lazy.webui.js`.

Activation and prefetch share one load per URL, with a 10 second deadline so a
stalled request fails and the next activation retries. Every load also starts
the region's scripts: one in-flight import per tag, skipped for tags already
declared. Settled successes are removed instead of retained in a fulfilled-load
cache. A failed loader retains only a retry marker so a declaration made before
its resource preparation failed cannot bypass that failure on the next load.
Synchronous callback exceptions reject through the same activation error path.
A callback must declare its tag before resolving; a pending WebUI declaration
waiting for template registration counts, but a resolved no-op does not.
Invoking a callback again does not clear native module-cache failures, and
template release never unloads JavaScript. In link builds it
also preloads the stylesheets the container's `lc` lists for the region,
resolved against the asset base, at the load's fetch priority, so they download
in parallel with the pack instead of after it arrives. A region mounts
only after its pack is installed and its scripts have resolved, so its custom
elements are defined before the body is inserted. A body that arrives open, in
SSR or a route partial, has its templates but not its scripts; they start while
it hydrates, and a later toggle waits for them. A region whose pack is
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
through the registration event; joins its definition and style readiness; and
then merges the pack's inventory bits only if the publication's epoch is still
current. Collection during either awaited step invalidates that acknowledgement
for the entire pack, including ordinary dependencies, even if a weak cache lookup
still finds its root. Reusable document-owned styles remain installed; suppressing
an old acknowledgement does not release them. Failed readiness leaves the pack
unready and unacknowledged so the next activation retries.

Triggers arm when the block containing a closed region renders or hydrates:
intent through one delegated document listener for pointer, focus, and touch
events whose composed path includes an element with the owner's tag; idle in an
idle callback after the window load event; eager in the task after hydration.
A pointer or focus move between two elements of one shadow tree is not
dispatched past that tree's host, so the document never sees it. Each owner
host with regions waiting for intent gets the same passive listeners after
mounting, hydration, or a structural update that reveals another block. An
event that enters the host's subtree from outside always reaches the host.
Repeated registration does not accumulate listeners. Once the owner's regions
are prefetched, those listeners find nothing to do.
Demand regions load only on activation or explicit prefetch. When arming finds
the focused element (followed through shadow roots) already inside an element
with the owner's tag, as with `autofocus`, that focus is not a gesture: the
owner's regions prefetch in the background once the window has loaded and the
browser is idle, unless an intent event arrives first. Idle and eager
prefetch yield to `navigator.connection.saveData` and skip regions that were
already released. Every speculative load (eager, idle, intent, and `$prefetch`)
waits while `document.prerendering` is true and runs on `prerenderingchange`;
activation never waits. Eager, idle, and background focus prefetches use fetch
priority `low`; intent and `$prefetch` use `low` until the document is complete
and `high` after it; activation uses `high`. Bundler `import()` takes no
priority and Chromium raises stylesheet preloads to at least high, so a
speculative load before the document is complete fetches only its pack and
starts the region's scripts and stylesheets at the window `load` event; its
readiness does not wait for that deferred work. Prefetch failures are silent;
activation surfaces errors and retries.

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

Setting a lazy condition is the whole activation API. `lazy.js` adds one
optional method to elements and to their types through module augmentation.
`$prefetch(path?)` loads, without rendering, the regions whose conditions are in
the element's own template, all of them (including demand regions) or only
those whose compiled condition paths equal `path` or lie below it; it returns
nothing and its failures are silent. The core's `$whenRendered()` waits for
captured lazy activations in the component and affected descendants, including
nested regions and the scripts of open bodies. It rejects with a pack or script
failure and retries failed regions that remain active when called again.

While a region remains active, an activation failure dispatches `lazyerror` on
the owner: bubbling, composed, cancelable, with `detail.error`. If no listener
cancels it, the error goes to `reportError` (or `console.error`). Internal
promises never become unhandled rejections. Closing or disposing a region
retires its activation; a late failure
does not dispatch an error for that activation or affect a replacement. Shared
loads still report failures to any activations that remain active.

Development builds add two warnings, each issued once. After the
page loads and goes idle, the runtime warns for each tag in the script map that
is already defined or declared (waiting for its template) although no region
that lists it has requested its scripts, which indicates a static import in the
startup bundle. When a pack registers scripted components that have no loader
and are neither defined nor declared, it warns that the application imported
`lazy.js` instead of the generated module.

`resolveAsset` reads the base from `window.__webui.assetBase`, else from the
document's `<meta name="webui-asset-base">`, else uses the document base URL.

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
