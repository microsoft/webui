---
title: "Hydration"
description: "Connect browser behavior to server HTML, choose deferred or streaming activation, and configure state projection and Trusted Types."
---

# Hydration

Hydration attaches browser bindings and listeners to existing server-rendered
HTML. Import your authored component modules from a browser entry to register
them; static SSR does not require the framework.

| Component files | Browser behavior |
|-----------------|------------------|
| `user-card.html` + `user-card.ts` or `user-card.js` | The authored class owns events, lifecycle, reactive state, and imperative APIs |
| `user-card.html` only | The server-rendered HTML stays inactive unless later navigation or state changes require browser rendering |

Use [Interactivity](/learn/interactivity) to write a component class.
This page covers when its behavior becomes ready and which state reaches it.

Build and deploy `protocol.bin` and the browser bundle with matching WebUI
versions. Rebuild both after upgrading.

Text bindings retain their surrounding authored whitespace on the server and
during browser updates. Values are not trimmed; ordinary HTML whitespace
collapsing and CSS such as `white-space: pre-wrap` determine their appearance.
Native HTML rules still apply, including ignoring the first newline immediately
after an opening `<pre>` or `<textarea>` tag.

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
remove, or reorder its server-rendered nodes.

Components using `@event` must be authored because the compiler needs a real
handler implementation. Do not add an empty class merely to make template
bindings or routing work.

<a id="lazy-hydration"></a>

## Deferred Hydration Lifecycle

Deferred policies keep the trusted SSR subtree in the DOM. They change when
browser rendering work and component bindings/listeners activate; they do not
defer HTML parsing, DOM construction, custom-element definition, or resource
discovery.

- Visibility policies activate shortly before the component enters the
  viewport or immediately before interaction.
- Interaction policy waits for pointer, focus, keyboard, or click intent and can
  keep the component module graph out of startup JavaScript and heap.
- Client-created components, successfully hydrated reconnects, and eager
  instance overrides mount eagerly.
- `hydratedCallback()` is the cross-policy signal for work that requires
  bindings or `w-ref` values.
- Missing optional coordinator/browser support falls back to eager hydration.

See the [Lazy Component Policy](/learn/directives/lazy) reference for
the complete policy matrix, required reservation syntax, combinations, and
per-instance overrides.

## Interaction-Triggered Application Hydration

Mark the application component:

```html
<template w-hydrate="interaction">
  <nav>...</nav>
  <outlet />
</template>
```

Then use the small router interaction entry instead of eagerly importing the
component graph:

```typescript
import {
  installInteractionHydration,
  wakeInteractionHydration,
} from '@microsoft/webui-framework/interaction-hydration.js';
import { routePreload } from '@microsoft/webui-delivery/router.js';

const preload = routePreload({ onIntent: () => wakeInteractionHydration() });
const disposeHydration = installInteractionHydration({
  onError: () => preload.destroy(),
  load: async () => {
    const [, { WebUIRouter, navigationCache }] = await Promise.all([
      import('./component-definitions.js'),
      import('@microsoft/webui-delivery/router.js'),
    ]);
    new WebUIRouter().start({ preload, cache: navigationCache(), loaders });
  },
});
try {
  preload.prepare();
} catch (error) {
  disposeHydration();
  throw error;
}
```

Mouse hover can prefetch a route. Pointer-down, focus, and keyboard interaction
start loading the application; an eligible click waits until it is ready.
The component-definitions entry imports `startRuntime` from `runtime.js`,
`styles.js` when needed, and its component modules, then calls `startRuntime()`.

This policy trades first-interaction latency for lower startup JS and heap, so
measure both. Prefer an eager root with `w-render="lazy"` descendants when
request-to-hydrated time matters. Synthetic replay cannot preserve transient
user activation or target closed-shadow controls; hydrate those paths eagerly.

For one offscreen singleton, interaction hydration can be combined with lazy
rendering. The boundary hydrates immediately after its module loads rather than
entering the visibility queue. See
[Lazy Rendering Until Interaction](/learn/directives/lazy#lazy-rendering-until-interaction)
for syntax and usage constraints.

Non-router apps can instead use the lower-level
`installInteractionHydration({ load })` entry. Its `load()` promise must mean all
target listeners are ready. Use `isInteractionReplay(event)` when ancestor
capture work must distinguish the synthetic replay.

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
[`<boundary>`](/learn/directives/boundary) where readiness changes:

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

For a manually authored early entry, import the coordinator before the
component registrations needed for early interactivity:

```typescript
import { startRuntime } from '@microsoft/webui-framework/runtime.js';
import '@microsoft/webui-framework/styles.js';
import '@microsoft/webui-framework/streaming.js';
import './ntp-page.js';
import './search-box.js';

startRuntime();
```

The host receives a descriptor when rendering reaches `search-ready`.
`resume` writes only that boundary, allowing `<search-box>` to hydrate once its
code and metadata are ready. `advance` continues through the parent section and
the next boundary or response end.

Boundaries can also occur in true conditions and selected route content. A
boundary-bearing subtree reached from a `<for>` body fails the build with
`boundary-in-repeat`. A complete `<for>` may instead sit inside one boundary,
and boundaries before or after a `<for>` are valid.

### Separate coordinator and application assets

The application must explicitly import the coordinator. To keep unrelated
application code out of `<head>`, author a small entry containing only:

```typescript
// src/streaming.ts
import '@microsoft/webui-framework/streaming.js';
```

Register this file as a normal entry in your bundler, alongside the application
entry. WebUI does not create entries, inject this import, or provide a streaming
build plugin. Streaming does not require a particular bundler:

- Emit the streaming entry separately from application startup, preserving its
  initialization side effect.
- Share framework modules between the entries rather than bundling duplicate
  registries or lifecycle state.
- Identify the coordinator output and its static dependencies through the
  bundler's entry metadata, not generated filenames. Keep application code and
  dynamic hydration imports out of its early preload list.

Use your bundler's existing output metadata and asset handoff to map those
outputs to deployed URLs. No separate WebUI streaming manifest is required.

Render the resolved URLs in your page:

```html
<head>
  <for each="href in streamingImports">
    <link rel="modulepreload" href="{{href}}">
  </for>
  <script type="module" async src="{{streamingUrl}}"></script>
</head>
<body>
  <ntp-page></ntp-page>
  <script type="module" src="{{applicationUrl}}" fetchpriority="low"></script>
</body>
```

The coordinator must execute while the document is still parsing, so use
`type="module" async` for its head script. A module without `async` waits for
parsing to finish. The mode marker keeps streamed roots deferred and prevents
premature completion if the application arrives first.

Do not globally inject the coordinator into every module. Load its entry
explicitly instead. Do not preload deferred application chunks; authored
`fetchpriority="low"` modules are excluded from WebUI's automatic modulepreload
hints. Footer placement alone does not disable those hints.

When you deliver an application entry, deliver any required static shared-chunk
`modulepreload` links alongside it. Otherwise the browser must fetch the entry
before discovering those chunks, adding a network round trip. For a deferred
entry, keep these links beside that entry in the body rather than moving them
to the head. Use your bundler's existing dependency metadata for their URLs.

Delaying every component definition also delays interactivity and keeps pending
state alive longer. Load registrations needed for early-interactive boundaries
when needed, and defer unrelated application startup. Compiler-owned scriptless
hosts can load framework support on demand without importing your application.

### Timing and lifecycle

An explicitly bounded child can hydrate before its unfinished parent.
Other descendants wait for the parent. Registration alone does not prove
readiness: streamed metadata and HTML must also have arrived.

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

An analytics script may load after early hydration events. Read the
performance timeline instead: WebUI emits a `performance.mark()` for each
commit without a flag or listener.

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

Use the host streaming API rather than constructing or modifying generated
records. Malformed, truncated, out-of-order, or over-budget streams fail closed,
log an error, and do not emit `webui:hydration-complete`. Keep compiler,
handler, and browser assets from compatible versions.

### CSP and delivery

Pass the request nonce with `RenderOptions::with_nonce`. The handler applies it
to generated inline boundary scripts and module-preload hints. Give authored
application module scripts and any host-authored `rel="modulepreload"` links
the same response nonce. Preloading without it can give an imported module graph
different CSP authorization than its entry script, including later CSS-module
imports. Keep the application's script and style policies in force; do not
allow `data:` scripts to compensate. Use a fresh nonce per response.

For an authored entry, supply `cspNonce` as trusted server state using the same
value passed to `with_nonce`:

```html
<script type="module" src="/index.js" nonce="{{cspNonce}}"></script>
```

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

## Trusted Types

WebUI automatically uses Trusted Types for native compiled templates and generated
CSS import maps when the browser supports them. There is no setup call or special
import order. WebUI creates its private policy, named **`webui`**, on first use.

The application decides whether to enforce Trusted Types through its response CSP:

```text
require-trusted-types-for 'script'; trusted-types webui
```

These directives supplement, not replace, your nonce-based `script-src` and
`style-src` policy. Pass the fresh response nonce to WebUI's renderer.
Nonces and Trusted Types solve different problems. WebUI does not create a
`default` policy or require `'unsafe-eval'`.

If CSP restricts policy names, it must allow `webui`, even when sink enforcement is
not enabled. Let WebUI create that policy and share one framework module across
bundles to avoid duplicate-policy rejection. Policy creation errors are reported,
not bypassed with strings. Browsers without Trusted Types keep normal rendering.
Client-side router partial navigation is not yet supported with enforced Trusted Types.

**This policy is not an HTML sanitizer.** Treat compiled template metadata and
component asset modules as executable application code; never populate them from
user HTML or request state. `registerTemplateData` is a trusted compiler-output
registration API, and registered HTML must remain immutable. Keep untrusted data
in normal escaped bindings.

Raw triple-brace state HTML is **not** promoted through the compiler policy.
Client raw-HTML updates with strings remain blocked under enforcement; a rejected
update leaves its previous DOM intact. FAST/string templates, arbitrary scripts,
event-handler attributes, and script URLs are not authorized by the compiler policy.
Trusted Types also does not sanitize HTML already delivered by the server.

## Build-Time State Projection

State projection is opt-in. Build your browser code with the projection plugin,
then pass the generated `webui-projection.json` to WebUI to reduce the state
sent to the browser.

Projection includes inherited `@observable` and `@attr` fields, including
those from statically analyzable class factories such as
`class Card extends withCounter(WebUIElement) {}`. Not every valid JavaScript
factory is analyzable: if the compiler cannot prove the exact state keys, the
projection build reports an error rather than dropping inherited state.

The supported esbuild plugin is exported from `@microsoft/webui/projection.js`:

<webui-press-tabs sync="package-manager" storage="local">
<webui-press-tab slot="tab" value="npm" active>npm</webui-press-tab>
<webui-press-tab slot="tab" value="pnpm">pnpm</webui-press-tab>
<webui-press-tab slot="tab" value="yarn">Yarn</webui-press-tab>
<webui-press-tab slot="tab" value="bun">Bun</webui-press-tab>
<webui-press-tab-panel active>

```bash
npm install -D esbuild typescript
```

</webui-press-tab-panel>
<webui-press-tab-panel>

```bash
pnpm add -D esbuild typescript
```

</webui-press-tab-panel>
<webui-press-tab-panel>

```bash
yarn add --dev esbuild typescript
```

</webui-press-tab-panel>
<webui-press-tab-panel>

```bash
bun add --dev esbuild typescript
```

</webui-press-tab-panel>
</webui-press-tabs>

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

Do not edit the generated manifest. Re-run the client build whenever its inputs
or outputs change, then rebuild the protocol.

Behavior is intentionally strict:

- With no manifest, the build remains correct and sends full state. Projection
  is disabled rather than guessed.
- Once any manifest is supplied, every scripted component compiled into the
  protocol must have exactly one entry. Missing coverage fails with
  `PROJ-B001`.
- Shared controls supplied through `--components` remain application-owned
  bundles. If they are external to the main bundle, build them separately and
  pass each manifest fragment with another `--projection-manifest`.
- Stale source inputs or outputs fail the WebUI build. Re-run the client
  bundler before rebuilding the protocol.
- `@attr` entries use JavaScript property names. During hydration, an existing
  SSR host attribute wins; projected state seeds the property only when that
  attribute is absent.

Use your existing esbuild entry points, splitting, and output naming. The plugin
runs as part of that build; it does not start a second bundle.

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

Projection is a payload optimization, not a secrecy boundary. Never place
credentials, private tokens, or other secrets in render state.

## Routing

The router and framework can mount HTML-only routes from compiled templates
without empty component classes. If the framework is not loaded and no authored
custom element owns the destination tag, navigation falls back to a full page
request.
