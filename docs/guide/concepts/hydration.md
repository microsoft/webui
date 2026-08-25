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
remove, or reorder nodes in its SSR subtree. Hydration numbers the trusted
server DOM to match the compiled template and cannot recover once that
numbering shifts.

Components using `@event` must be authored because the compiler needs a real
handler implementation. Do not add an empty class merely to make template
bindings or routing work.

## Lazy Hydration

For long feeds and grids, the recommended policy reduces all browser work that
can be deferred without removing the SSR DOM:

```html
<!-- activity-row.html -->
<template
  w-render="lazy"
  w-reserve-block-size="72px"
>
  <button @click="{open()}">{{label}}</button>
</template>
```

`w-render="lazy"` does two things:

1. It hydrates SSR instances only when the browser considers them relevant,
   within the 200px viewport lead, or immediately before interaction.
2. It emits `content-visibility: auto` and
   `contain-intrinsic-block-size: auto 72px` for the component tag in a
   document-level `<style>` before first layout, plus a tag-qualified component
   rule for Shadow DOM instances. A Light component nested inside an authored
   shadow root receives the rule through its precomputed style closure, which
   delivers the stylesheet into that root under every CSS strategy.

The required reservation should approximate one instance's rendered block size.
It preserves scroll geometry while the browser skips offscreen style, layout,
paint, and raster work, then lets `auto` remember the measured size after the
instance renders. Accepted values are single non-negative CSS lengths such as
`72px`, `18rem`, `40dvh`, or `25cqb`; percentages, `auto`, negative values, and
functions such as `calc()` are rejected at build time.

The policy wrapper is build-only. WebUI removes its policy attributes. A
component that authors `<template shadowrootmode="open">` keeps that wrapper as
its declarative shadow root; every other component is Light, so the wrapper is
unwrapped.

Import the optional entry once before registering policy-bearing components:

```typescript
import '@microsoft/webui-framework/lazy-hydration.js';
import './activity-row.js';
```

If rendering containment is not safe for a component, defer only hydration:

```html
<template w-hydrate="lazy">
  <button @click="{open()}">{{label}}</button>
</template>
```

WebUI keeps the SSR content visible and hydrates each instance within 200px of
the viewport or immediately on interaction. For the complete policy, Chromium's
`contentvisibilityautostatechange` relevance event becomes the primary signal
after the target reports a native state. The shared `IntersectionObserver`
remains a fallback when definition happens after the initial native event; other
browsers use it throughout. Client-created components remain eager. After a
successful mount, reconnect also stays eager and preserves current client state
rather than replaying SSR bootstrap state. Captured pointer hover activates a
pending component before its first `@mouseenter` handler.

Keep one priority instance's hydration eager while retaining rendering deferral:

```html
<activity-row w-hydrate="eager"></activity-row>
```

Disable both parts of the complete policy for one instance:

```html
<activity-row w-render="eager"></activity-row>
```

Use `hydratedCallback()` for setup that requires bindings or refs. If the
optional entry or `IntersectionObserver` is unavailable, WebUI falls back to
eager hydration. The rendering policy remains browser-managed. Keep the default
eager mode for small, fully visible groups.

The SSR subtree remains in the DOM and available to find-in-page and
accessibility tooling. The browser can make skipped content relevant for those
features and WebUI hydrates when the native relevance event fires. This policy
does not defer HTML parsing, DOM construction, custom-element definition, or
resource discovery. Use server streaming, pagination, virtualization, and native
resource hints when those costs dominate.

## Interaction-Triggered Application Hydration

An SSR page can defer its full component graph until the first user action:

```typescript
import {
  installInteractionHydration,
  isInteractionReplay,
} from
  '@microsoft/webui-framework/interaction-hydration.js';

const root = document.querySelector('todo-app');
if (root) {
  installInteractionHydration({
    root,
    load: () => import('./component-definitions.js'),
  });
}
```

Pointer-down, focus, and keyboard intent starts `load()` without cancellation.
An unmodified primary click waits and replays on its original composed-path
target; the promise must resolve only after listeners are ready. Hover, modified
clicks, and previously cancelled clicks do not replay. The returned disposer
removes the boundary, and `onError` receives loader failures. Use
`isInteractionReplay(event)` to deduplicate ancestor capture work.

This policy trades first-interaction latency for lower startup JS and heap, so
measure both. Prefer an eager root with `w-render="lazy"` descendants when
request-to-hydrated time matters. Synthetic replay cannot preserve transient
user activation or target closed-shadow controls; hydrate those paths eagerly.

### Images in deferred components

Visibility-deferred hydration delays JavaScript bindings, not image fetching. Use
`loading="lazy"`, `srcset`, `sizes`, and explicit image dimensions.

An `@load` or `@error` event may fire before a deferred component hydrates. If
component state depends on it, bind the image with `w-ref` and reconcile its
current status in `hydratedCallback()`:

```typescript
@observable imageState = 'pending';
image!: HTMLImageElement;

protected override hydratedCallback(): void {
  if (this.image.complete) this.updateImageState();
}

updateImageState(): void {
  this.imageState = this.image.naturalWidth > 0 ? 'loaded' : 'error';
}
```

Call the same idempotent method from `@load` and `@error`.

## Progressive Streaming Hydration

WebUI can hydrate a complete child region while its reusable parent component
is still rendering. Author a
[`<boundary>`](/guide/concepts/directives/boundary) where readiness changes:

```html
<!-- index.html -->
<head>
  <script type="module" async src="/index.js"></script>
</head>
<body>
  <ntp-page></ntp-page>
</body>
```

```html
<!-- ntp-page.html -->
<main>
  <h1>{{title}}</h1>
  <boundary name="search-ready">
    <search-box query="{{query}}"></search-box>
  </boundary>
  <section>{{slowFeed}}</section>
</main>
```

Import the coordinator before registrations:

```typescript
import '@microsoft/webui-framework/streaming.js';
import './ntp-page.js';
import './search-box.js';
```

The entry uses one `<ntp-page>`. When traversal reaches its internal boundary,
WebUI pauses and returns a runtime descriptor to the host. `resume` commits only
`<search-box>` through its checkpoint, so it can become interactive
immediately. `advance` then renders the remaining parent section, generated
span completion, and later shell bytes. This boundary-only resume means a
sibling boundary is not needed to separate the early child from the parent
tail.

Boundaries can also occur in true conditions and selected route content. A
boundary-bearing subtree reached from a `<for>` body fails the build with
`boundary-in-repeat`. A complete `<for>` may instead sit inside one boundary,
and boundaries before or after a `<for>` are valid.

### Timing and lifecycle

When a boundary pauses inside a component, WebUI generates a span around the
unfinished parent. The early child is compiler-marked to bypass exactly that
nearest parent barrier. Other descendants stay opaque until the parent span
completes. The same bounded traversal works in light DOM and across open shadow
roots and slots.

When a component calls `.define()` before streamed template metadata arrives,
WebUI delays native definition because browsers snapshot `observedAttributes`
at definition time. When a checkpoint arrives first, undefined roots share one
definition waiter per tag. Parents hydrate before ordinary descendants.

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
  `{ sequence, terminal, kind }`. Sequence numbers are response order, not
  authored names. Keep this diagnostics flag off in production.
- `webui:hydration-complete` once the empty terminal record has arrived and no
  eager component, checkpoint, generated span, definition waiter, or ancestor
  barrier remains pending. It means the complete response lifecycle is done,
  not merely that the first interactive child is ready.

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
| `webui:span:<id>` | A generated parent span completes |
| `webui:streaming:terminal` | The terminal record settles |

Because marks sit in the performance timeline, they can be read at any later
point:

```js
const commits = performance
  .getEntriesByType("mark")
  .filter((entry) => entry.name.startsWith("webui:"));
```

Boundary `<id>` values are response-local occurrence IDs, not declaration IDs
or authored names. Span IDs use a separate response-local namespace.

### Hydrating across several tasks

By default the coordinator drains its queue in one pass, which reaches
interactivity soonest. That assumes boundaries arrive spread across the
response. If an intermediary buffers and coalesces the response, records can all
arrive at once and hydrate in a single long task - exactly what streaming is
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

After a checkpoint commits, WebUI removes its generated payload, sentinel,
markers, and temporary attributes. Final occurrences release roots immediately.
Updatable occurrences retain only live roots and a pending shallow patch until
terminal.

The browser protocol is `[2, sequence, kind, target, payload]`. Kinds are final
checkpoint, updatable checkpoint, update, generated span completion, and
terminal. A malformed, truncated, out-of-order, or over-limit stream fails
closed, suppresses successful completion, and releases discoverable deferred
state within fixed bounds.

At `body_end`, the handler emits one markerless empty terminal envelope:
`[2,nextSequence,4,0,{}]`. Its flush also commits any preceding native or static
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

### Updates

Commit an occurrence as updatable only when complete SSR markup should become
interactive before its slow state resolves. `update(instanceId, patch)` applies
projected state through normal reactivity. It never inserts or replaces markup
and never reruns hydration. If the class or parent barrier is still pending,
WebUI queues one shallow patch and applies it after successful activation.

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
