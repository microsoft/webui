# Hydration

WebUI renders components reached by the initial request on the server.
JavaScript is optional:

| Component files | Browser behavior |
|-----------------|------------------|
| `user-card.html` + `user-card.ts` or `user-card.js` | The authored class owns events, lifecycle, reactive state, and imperative APIs |
| `user-card.html` only | The server-rendered HTML stays inactive unless later navigation or state changes require browser rendering |

This keeps first-page work small without requiring empty TypeScript classes.

## HTML-Only Components

HTML-only components can use bindings, attributes, `<if>`, and `<for>`. Their
initial server-rendered DOM needs no hydration work or browser state.

When the framework is loaded, it can later activate the compiled template for
soft navigation or a browser-applied state update. Client-created instances
mount immediately. Existing repeated content remains in place until its
collection is explicitly supplied; supplying an empty array removes it.

An app that remains static after SSR does not need the framework. An app that
wants HTML-only soft navigation or browser-applied template updates imports
`@microsoft/webui-framework` once in its browser entry.

## Authored Components

Add a sibling module only when the component owns browser behavior:

```typescript
import { WebUIElement } from '@microsoft/webui-framework';

export class UserCard extends WebUIElement {
  // Events, lifecycle, decorators, or imperative APIs belong here.
}

UserCard.define('user-card');
```

Only `@observable` and `@attr` fields are eligible for exact initial state
projection. Ordinary template values already exist in the rendered HTML and do
not need to enter browser state just because the component has an event handler,
`w-ref`, lifecycle method, or imperative API.

An authored component with no decorators can therefore wire its behavior
without adding any startup state.

For a normal buffered page, load authored component definitions with a
parser-inserted, non-async ES module script or a classic `defer` script. If a
classic script blocks parsing, place it after every SSR instance it may upgrade.
This guarantees that each component subtree exists before upgrade. WebUI then
hydrates synchronously inside `super.connectedCallback()`; when it returns,
bindings, events, and `w-ref` references are ready. Progressive streaming pages
use the early async module contract described below instead.

Until a containing WebUI component hydrates, descendants must not insert,
remove, or reorder nodes in its SSR subtree. Hydration resolves compiled paths
against the trusted server DOM and cannot recover after those paths shift.

Components using `@event` must be authored because the compiler needs a real
handler implementation. Do not add an empty class merely to make template
bindings or routing work.

## Lazy Authored Components

Opt an authored component into per-instance lazy hydration with
`static lazy = true`:

```typescript
export class ActivityRow extends WebUIElement {
  static lazy = true;
}

ActivityRow.define('activity-row');
```

The SSR DOM stays visible and keeps its identity. The framework uses one shared
`IntersectionObserver` for all lazy instances and hydrates an element when it
enters a 200px lead area around the viewport. Supporting browsers use a 200px
observer `scrollMargin` with zero `rootMargin`, extending the same lead through
nested scroll containers without doubling it at the document root. Older
observers use a 200px `rootMargin`; an item inside a nested scroller activates
when it reaches that scroller's clip boundary.

Pointer, focus, keyboard, and click capture activates a pending component
synchronously before normal target handling. Nested lazy components activate
parent-first across slots and shadow roots. If a visible batch exceeds an 8ms
work budget, WebUI continues it in a user-visible scheduler task, or a
`MessageChannel` task where the Scheduler API is unavailable. A rejected
scheduler task uses the same fallback without dropping visible work.

Lazy hydration applies only to SSR instances. Client-created components mount
eagerly. If `IntersectionObserver` is unavailable, SSR hydration also falls back
to eager behavior rather than leaving inert UI.

After a hydrated component stays disconnected long enough for teardown,
structure-free templates rewire the preserved DOM on reconnect. Templates with
conditionals or repeats remount from the component's current state because their
one-time SSR closing markers have already been removed.

Writes made after `super.connectedCallback()` has placed a lazy instance in its
deferred state are retained. They take precedence over older bootstrap state and
are replayed synchronously after hydration, including parent-to-child writes
from repeated lists and explicit same-value assignments. Ordinary bootstrap
values are copied into component-local state before a router can release the
global handoff. If one replayed binding also references template-only state that
was omitted from bootstrap, WebUI preserves the complete trusted SSR binding
during replay and later updates until that missing root is supplied. An explicit
`undefined` counts as supplied state and clears the corresponding text or
repeat. This does not change the ordinary
pre-hydration mismatch rule: writes made in a field initializer, constructor, or
before `super.connectedCallback()` still cannot change the trusted first server
paint.

On a progressive streaming page, a boundary commit records its local state and
removes its temporary scaffolding without forcing a lazy root to hydrate.
Subsequent boundary patches update the dormant root, and the newest values win
when visibility or interaction activates it.

Use lazy hydration for long feeds, grids, and other pages where most authored
components begin outside the lead area. For a small page whose components are
all visible, eager hydration avoids the observer-delivery delay.

## Progressive Streaming Hydration

WebUI can hydrate an explicit, complete entry-page region while the document
is still loading. Author a
[`<boundary>`](/guide/concepts/directives/boundary), then serve the page
with the Rust `WebUIHandler::render_streaming` API or a versioned backend
control stream through `webui serve --api-port`.

```html
<head>
  <!-- Must be ahead of boundary content and able to run during parsing. -->
  <script type="module" async src="/index.js"></script>
</head>
<body>
  <header>
    <boundary name="weather-shell">
      <weather-panel status="loading"></weather-panel>
    </boundary>
  </header>

  <boundary name="critical-composer">
    <message-composer></message-composer>
  </boundary>

  <boundary name="low-priority-feed">
    <activity-feed></activity-feed>
  </boundary>
</body>
```

Import the streaming entry before component registration modules:

```typescript
import '@microsoft/webui-framework/streaming.js';
import './weather-panel/weather-panel.js';
import './message-composer/message-composer.js';
import './activity-feed/activity-feed.js';
```

The streaming entry installs the coordinator synchronously. It is separate from
the default framework entry so non-streaming applications do not download,
parse, or initialize streaming code. The application module must use `async`,
or an equivalent non-blocking loading strategy, in `<head>` before the first
boundary. A normal module script is deferred until parsing completes and
defeats early hydration. The parser currently validates boundary syntax and
placement, but does not validate this script loading order.

The server commits boundary HTML in document order. In this example the weather
shell commits first as an updatable boundary, so it never delays the critical
composer. The host starts forecast work concurrently and sends a projected state
record to the weather boundary whenever it resolves, including between feed
checkpoints.

The state record uses the original open HTML response. It invokes component
reactivity without rerunning hydration or `hydratedCallback()`, and it does not
replace or relocate server markup. If the weather class is still downloading,
WebUI merges the patch into its pending activation state and activates once with
the newest values. This keeps the critical island's time to interactive
independent of the slow surface without a client fetch.

See `examples/app/streaming` for a complete working version of this pattern.

Every registered WebUI component rendered through `render_streaming` must be
inside an explicit boundary. Native HTML and unregistered static tail markup can
remain outside. This lets the handler mark each streamed SSR component before
custom-element upgrade and guarantees that a later checkpoint can activate it.
The checkpoint also includes metadata and projected state for descendants that
are reachable inside those roots but initially hidden by a false condition or
empty repeat. It does not include unrelated components rooted in later
boundaries, and its inventory marks only SSR roots that actually rendered.

### Timing and lifecycle

When a component calls `.define()` before its streamed template metadata
arrives, WebUI delays the native custom-element definition. Browsers snapshot
`observedAttributes` at definition time, so defining early would permanently
lose template-derived attribute observation. When a boundary arrives before its
component module, it waits on one custom-element definition reaction per tag.
An undefined outer root is an activation barrier for its descendants. Once the
outer definition and metadata are ready, WebUI hydrates parent first, then
descendants, without waiting for `DOMContentLoaded`.

Use `hydratedCallback()` for setup that needs bindings, events, or `w-ref`
references. It runs synchronously exactly once after the first successful
ordinary hydration, client mount, streamed activation, or dormant static-host
wake, including a lazy activation. `connectedCallback()` can run earlier on a
lazy or deferred streamed host and can run again after reconnect, so it is not a
universal post-hydration signal. Actual timing still depends on module download,
visibility, server progress, and transport delivery.

WebUI dispatches these events on `window`:

- `webui:boundary-hydrated` after each commit, only when
  `window.__WEBUI_STREAMING_DEBUG__ === true`. Its `CustomEvent.detail` contains
  `{ sequence, terminal, kind }`, where `kind` is `"checkpoint"`, `"update"`, or
  `"terminal"`. Sequence numbers are response order, not authored boundary
  names. Keep this diagnostics flag off in production to avoid one event
  allocation per commit.
- `webui:hydration-complete` once the empty terminal record has arrived and no
  eager component or boundary remains pending. On a streaming page, it means the
  complete response hydration lifecycle is done, not merely that the first
  interactive boundary is ready. On ordinary parser startup, WebUI waits through
  `DOMContentLoaded` and the first intersection result for each lazy root.
  Initially visible roots finish before the event; roots classified as dormant
  do not keep it open and do not redispatch it when they activate later.

### Measuring commits in production

Events require a listener installed before the first commit, which is often
impossible: the coordinator is a separate async entry, so an analytics or RUM
script can easily load after early boundaries have already hydrated. WebUI
therefore also emits a `performance.mark()` for every commit, with no flag and
no listener:

| Mark | Emitted when |
| --- | --- |
| `webui:boundary:<id>` | A checkpoint commits |
| `webui:boundary:<id>:update` | A projected state update is applied |
| `webui:streaming:terminal` | The terminal record settles |

Because marks sit in the performance timeline, they can be read at any later
point:

```js
const commits = performance
  .getEntriesByType("mark")
  .filter((entry) => entry.name.startsWith("webui:"));
```

`<id>` is the integer boundary ID, not the authored name — name strings never
reach the response. The ID is the boundary's declaration index, so your build
manifest maps it back to the authored name offline.

### Hydrating across several tasks

By default the coordinator drains its queue in one pass, which reaches
interactivity soonest. That assumes boundaries arrive spread across the
response. If an intermediary buffers and coalesces the response, they can all
arrive at once and hydrate in a single long task — exactly what streaming is
meant to avoid.

Set a millisecond budget to make the coordinator yield to the browser between
boundaries instead:

```js
window.__WEBUI_STREAMING_SLICE_MS__ = 5;
```

Set it before the application entry runs. It trades total hydration time and
the last boundary's interactivity for responsiveness during hydration, so leave
it unset unless you have measured a long task. Record order and every
correctness guarantee are unchanged.

After a checkpoint commits, WebUI removes its generated payload, sentinel, and
marker nodes, plus the temporary streamed-host identity. Final boundaries
release their root list immediately. Updatable boundaries retain only their root
references and latest shallow patch until the terminal record. Boundary-local
state is never copied into `window.__webui.state`. Applications should not query
or depend on generated scaffolding.

At `body_end`, the handler emits one markerless empty terminal envelope:
`[1,nextSequence,3,0,{}]`. Its flush also commits any preceding native or static
tail HTML, but terminal records never repeat template metadata or state. A
truncated or malformed stream, or one exceeding a client work bound such as the
queued-boundary or marker-scan limit, logs an error, suppresses
`webui:hydration-complete`, and releases discoverable deferred state within
fixed bounds. Valid commits perform no document-wide scan; a bounded sweep is a
fatal-cleanup fallback only.

The client trusts records past three checks, because the same WebUI version
wrote them: `JSON.parse` (which alone detects any truncation, since a cut-off
record is never valid JSON), a five-element array, and the envelope `version`.
Everything else is enforced where it is actually knowable — a sequence or
boundary-target mismatch halts the stream, and a defective payload fails the
commit closed. Unrecognized *additive* payload fields are ignored rather than
fatal, so a cached older bundle keeps working against a newer server; anything
incompatible bumps `version` instead.

### CSP and delivery

Pass the request nonce with `RenderOptions::with_nonce`. The handler applies it
to generated inline boundary scripts, while your Content Security Policy must
also allow the external application module. Use a fresh nonce per response.

`FlushWriter::flush` means that WebUI handed all currently buffered bytes to the
HTTP transport. A server adapter, compression layer, CDN, or reverse proxy can
still buffer those bytes. Disable response buffering where appropriate and
verify early delivery through the production path.

### Current limits

Progressive streaming hydration is exposed by the Rust handler and browser
coordinator. Boundary markup is strictly in authored order, while markerless
state records can interleave. The following are not implemented APIs:

- Dynamic `<webui-stream>`, `page.append()`, or
  `begin_append()` / `commit()` APIs
- Out-of-order same-response replacement
- Streaming reuse by router partial navigations
- Node, FFI/.NET, WASM, or other host-language response sessions
- Declarative partial-update APIs

## Build-Time State Projection

Exact state projection is opt-in. Rust does not inspect JavaScript or
TypeScript. The application bundles its browser code first, and a bundler
adapter emits `webui-projection.json` from the same resolved graph and output
membership that produced the browser chunks.

The projection compiler contract is bundler-neutral. The
`@microsoft/webui/projection.js` subpath currently includes the supported
esbuild adapter:

```bash
npm install -D esbuild typescript
```

```js
// build-client.mjs
import * as esbuild from 'esbuild';
import { esbuildProjection } from '@microsoft/webui/projection.js';

await esbuild.build({
  entryPoints: ['src/index.ts'],
  outdir: 'dist',
  bundle: true,
  splitting: true,
  format: 'esm',
  plugins: [esbuildProjection()],
});
```

Run the client build once, then give its manifest to WebUI:

```bash
node build-client.mjs
webui build ./src \
  --plugin=webui \
  --projection-manifest ./dist/webui-projection.json \
  --out ./dist
```

The generated file has this shape (hashes abbreviated):

```json
{
  "schema": "webui.state-projection/v1",
  "producer": {
    "name": "@microsoft/webui/projection.js",
    "version": "0.0.18"
  },
  "adapter": {
    "name": "esbuild",
    "bundler": "esbuild@0.28.1"
  },
  "root": "..",
  "analysisHash": "sha256:...",
  "buildId": "sha256:...",
  "outputs": {
    "dist/index.js": "sha256:..."
  },
  "inputs": {
    "src/user-card.ts": "sha256:..."
  },
  "components": {
    "user-card": {
      "module": "src/user-card.ts",
      "outputs": ["dist/index.js"],
      "hydrationKeys": ["displayName", "selected"],
      "navigationKeys": ["displayName", "selected"]
    }
  }
}
```

Do not hand-author this file. It is a deterministic record of the completed
bundle and becomes stale as soon as a declared input or output changes.

The manifest records exact input hashes, emitted output hashes, code-split
membership, component ownership, and sorted `@observable` plus `@attr` property
names. WebUI validates those hashes and embeds only the resulting key surfaces
in `protocol.bin`. Runtime handlers do not load the manifest, TypeScript, or a
bundler.

Behavior is intentionally strict:

- With no manifest, the build remains correct and sends full state. Projection
  is disabled rather than guessed.
- Once any manifest is supplied, every scripted component compiled into the
  protocol must have exactly one entry. Missing coverage fails with
  `PROJ-B001`.
- Shared controls supplied through `--components` remain application-owned
  bundles. If they are external to the main bundle, build them separately and
  pass each manifest fragment with another `--projection-manifest`.
- Stale inputs or outputs fail the WebUI build. Re-run the client bundler before
  rebuilding the protocol.
- `@attr` entries use JavaScript property names. During hydration, an existing
  SSR host attribute wins; projected state seeds the property only when that
  attribute is absent.

The adapter runs inside the application's existing esbuild invocation. It does
not start a second bundler run, and it does not constrain chunking, dynamic
imports, external modules, or output naming.

Other bundlers are not coupled to esbuild. A Vite, Rollup, Rolldown, webpack,
Rspack, or other adapter can construct the exported `AdapterContext`, call
`compileProjection()`, and run the exported conformance suite. The official
package currently ships and supports the esbuild adapter.

## State Sent to the Browser

With validated projection manifests, the initial page includes only
`@observable` and `@attr` values needed by authored components on the active
route. Template values used only for server rendering stay out of browser
state. Without manifests, WebUI preserves full state for compatibility and
correctness.

Later soft navigations include the values needed to render the destination
components. Inactive sibling routes do not enlarge either payload. If the
initial page needs no client state, WebUI writes:

```json
{"state":{}}
```

State sent to the browser is client-visible. Never place credentials, private
tokens, or other secrets in it.

## Routing

The router and framework can mount HTML-only routes from compiled templates
without empty component classes. If the framework is not loaded and no authored
custom element owns the destination tag, navigation falls back to a full page
request.
