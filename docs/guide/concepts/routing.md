---
title: "Routing"
description: "Declare nested routes in HTML and add browser navigation, loaders, actions, cache invalidation, and pending or error states."
---

# Routing

Declare routes in HTML and place `<outlet />` where a child page belongs.
The native renderer matches the request path and returns the selected
components. Add `@microsoft/webui-delivery` for client-side navigation that keeps
unchanged parent components mounted.

Cache tags, pending UI, and error components are optional route attributes
validated at build time. Their browser behavior runs in the router, not in the
server's JavaScript engine.

## Installation

```bash
npm install @microsoft/webui-delivery
```

Only needed when your app has client-side navigation. Server-only apps with full page loads don't need it.

## Quick Start

**1. Add `<base href="/">` in your `<head>`:**

```html
<head>
  <meta charset="utf-8">
  <base href="/">
</head>
```

For a root-hosted app, this keeps relative asset URLs anchored at `/` rather
than at the current route. For a subpath deployment, use that prefix instead,
such as `<base href="/my-app/">`. Keep the host's route and asset paths aligned
with it.

**2. Declare routes in `index.html`:**

```html
<body>
  <route path="/" component="app-shell">
    <route path="" component="home-page" exact />
    <route path="users" component="user-list" exact />
    <route path="users/:id" component="user-detail" exact />
  </route>
  <script type="module" src="index.js"></script>
</body>
```

**3. Use `<outlet />` in your shell component:**

```html
<!-- app-shell.html -->
<nav><a href="/">Home</a> <a href="/users">Users</a></nav>
<main><outlet /></main>
```

The paired form `<outlet></outlet>` is also supported. Outlet directives are
empty; prefer the shorter self-closing form. Author only one outlet at a route
level, including outlets inside nested layout components and directives.
Child routes have their own outlet level. Additional outlets produce a
`multiple-outlets` build warning; move
duplicated layout into the matched child component instead.

**4. Start the router:**

```typescript
import { WebUIRouter } from '@microsoft/webui-delivery/router.js';

const router = new WebUIRouter();
router.start();
```

The server renders the matched route on first load. The router intercepts
eligible internal links for partial navigation; unsupported destinations can
fall back to a full document request.

Route segments may contain literal dots, such as `/docs/v2.1` or
`/users/john.doe`. When writing a custom host, use the
[server contract](#server-contract) below to distinguish page requests from
client navigation.

The router never imports framework code. Authored route components use their
registered classes. When the application calls `startRuntime()` from
`@microsoft/webui-framework/runtime.js`, HTML-only route templates can mount during soft
navigation without empty TypeScript modules. The router falls back to a full
document request only when no runtime registers the destination tag.

## Selecting router capabilities

There is one router class. Configure it once in `start()`; the constructor takes
no configuration and importing the package does not create a singleton.

```typescript
import { WebUIRouter } from '@microsoft/webui-delivery/router.js';

const router = new WebUIRouter();
router.start();
```

This router requests JSON and has no automatic preload, mutation interception,
navigation cache, pending/error presentation, or view transitions. URL/history,
server-authoritative route chains, outlets, and component loaders work normally.

Import only the capabilities you select:

```typescript
import {
  WebUIRouter, navigationCache, routeActions, routeBoundaries, viewTransitions,
} from '@microsoft/webui-delivery/router.js';
import { ndjson } from '@microsoft/webui-delivery/resources.js';

const router = new WebUIRouter();
router.start({
  transport: ndjson(),
  boundaries: routeBoundaries,
  transitions: viewTransitions,
  cache: navigationCache({ maxEntries: 20, staleTime: 10_000 }),
  actions: routeActions(),
});
```

| Capability | `start()` option |
| --- | --- |
| NDJSON partials | `transport: ndjson()` from `resources.js` |
| Pending/error UI | `boundaries: routeBoundaries` |
| View transitions | `transitions: viewTransitions` |
| Navigation cache | `cache: navigationCache(options)` |
| Mutation actions | `actions: routeActions()` |
| Intent preload | `preload: routePreload(options)`, with a configured cache |
| Development warnings | `diagnostics: routeDiagnostics` |

The route policies are named exports from `router.js`. Selecting a configured
policy enables it; there is no second boolean flag. Unselected implementations
are excluded by production tree-shaking. Selected event listeners are ready
before `start()` returns. Load application components lazily through `loaders`,
not by changing global configuration during navigation. Do not mutate loader
maps after startup. Calling `start()` on an active router is an error; destroy
it before restarting.

The NDJSON adapter is ready when imported; it does not wait until the first
response to import its decoder. JSON-only applications do not import it at all.
It bounds each UTF-8 record to 256 KiB and releases cancelled readers. A partial
request's deadline includes the deferred state tail. Malformed or truncated
tails are reported and never marked complete in the navigation cache.

For custom delivery code, `resources.js` exports `fetchComponentTemplates()`,
`registerTemplatesAndStyles()`, `readJsonPartial()`, and
`waitForTemplateReadiness()` without importing a router or renderer.
These are resource/transport primitives, not a new component-state
endpoint or a replacement for declaring compiled components.

With the transition capability selected, the router uses the View Transitions
API when available. Define transition styles in CSS; do not wrap
`router.navigate()` in another view transition. Navigation does not wait for the
animation to finish.

## Nested Routes

Routes nest to any depth. Each parent component uses `<outlet />` where its child route renders:

```html
<!-- index.html -->
<route path="/" component="app-shell">
  <route path="" component="dashboard-page" exact />
  <route path="sections/:id" component="section-page">
    <route path="topics/:topicId" component="topic-page">
      <route path="lessons/:lessonId" component="lesson-page" exact />
    </route>
  </route>
</route>
```

```html
<!-- section-page.html -->
<h2>{{sectionName}}</h2>
<nav>topic links...</nav>
<outlet />
```

When navigating between child routes, **parent content is preserved**. Navigating from `/sections/1/topics/react` to `/sections/1/topics/css` only remounts the topic component - the section heading and nav stay.

### The `exact` Attribute

Use `exact` on **leaf routes** - routes with no children. Without `exact`, a route matches any URL that starts with its path, which is what you want for parent routes that have `<outlet />`.

```html
<route path="/" component="app-shell">
  <route path="" component="home-page" exact />        <!-- leaf: exact -->
  <route path="users" component="user-list" exact />   <!-- leaf: exact -->
  <route path="settings" component="settings-page">    <!-- parent: NO exact -->
    <route path="profile" component="profile-page" exact />
    <route path="billing" component="billing-page" exact />
  </route>
</route>
```

**Rule of thumb:** If a route has `<outlet />`, don't add `exact`. If it doesn't, add `exact`.

### The `query` Attribute

Declare which URL query parameters are forwarded as HTML attributes on the component:

```html
<route path="compose" component="compose-page" query="action,to,subject" exact />
```

Navigating to `/compose?action=reply&to=user@test.com` sets `action` and `to` as attributes on `<compose-page>`. Unlisted params (e.g. `?class=evil`) are silently dropped.

| Behavior | Description |
|----------|-------------|
| No `query` attribute | No query params forwarded (deny-by-default) |
| `query="action,to"` | Only `action` and `to` set as attributes |
| Collision with path param | Path param wins — query param is skipped |

Declare `@attr` properties in the component to receive them:

```typescript
export class ComposePage extends WebUIElement {
  @attr action = '';
  @attr to = '';
  @attr subject = '';
}
```

### The `keep-alive` Attribute

Preserve a component's DOM and state across navigations instead of destroying and recreating it:

```html
<route path="/" component="app-shell">
  <route path="./" component="mail-view" keep-alive>
    <route path="" component="inbox-page" exact />
  </route>
  <route path="calendar" component="calendar-page" exact keep-alive />
  <route path="settings" component="settings-page" exact />
</route>
```

When navigating from Mail to Calendar and back:
- **`mail-view` (keep-alive):** Hidden on deactivation, shown instantly on return. The folder pane, email list, and all local state survive the round trip. Route param and query param attributes are updated, and your component's `@observable` properties are preserved.
- **`settings-page` (no keep-alive):** Destroyed on deactivation, recreated fresh on each visit.

| Behavior | With `keep-alive` | Without |
|----------|-------------------|---------|
| Deactivate | `display: none` (stays in DOM) | `display: none` (stays in DOM) |
| Reactivate | Reuses existing component — params updated, state preserved | Destroys old, creates new component |
| Local state | ✅ Preserved (scroll, input, timers, observables) | Lost |
| Server state | **Skipped** - use a [loader](#route-loaders) to refresh | Loaded with the new component |

<webui-blockquote appearance="tip" title="When to use" icon="💡">

Use on routes with expensive UI (lists, grids, trees) that users switch between frequently. Leaf routes with simple data-driven content rarely benefit — they're cheap to recreate.

</webui-blockquote>

<webui-blockquote appearance="tip" title="Refreshing data on reactivation" icon="💡">

If a keep-alive component needs fresh data when reactivated, define a static `loader()` method. The router calls it on every navigation, including reactivation, and uses the returned data to refresh the component.

</webui-blockquote>

### Preload on Hover

Prefetch route data when a mouse hovers an internal link to reduce the wait
after a click:

```ts
import { navigationCache, routePreload } from '@microsoft/webui-delivery/router.js';

router.start({ cache: navigationCache(), preload: routePreload() });
```

Preloading applies to internal links hovered with a mouse. A ready, fresh
result avoids another fetch on click; hovering another link cancels the previous
preload. It is off by default, so enable it only when the navigation benefit
justifies speculative requests.

For interaction-deferred apps, create the same `routePreload({ onIntent })`
policy and call its `prepare()` method before activation. It starts one bounded
JSON prefetch without loading the router or a stream decoder. Pass that policy
and a configured cache to `router.start()` to adopt the request without another
fetch. Start framework-specific hydration from `onIntent`. WebUI Framework calls
`wakeInteractionHydration()` from that callback; FAST uses its own hydration
readiness signal. The router imports neither runtime.

### Route Loaders

Define a static `loader()` method on a component class to fetch data from a custom source instead of using server-provided state:

```typescript
import { WebUIElement } from '@microsoft/webui-framework';
import type { RouteLoaderContext } from '@microsoft/webui-delivery/router.js';

export class LiveDashboard extends WebUIElement {
  static async loader({ params, signal }: RouteLoaderContext) {
    const resp = await fetch(`/api/dashboard/${params.id}`, { signal });
    return resp.json();
  }
}
```

Loaders receive `params`, `query`, and an `AbortSignal`, and resolve before the
new view appears. Each route declaration keeps its own data, even when several
routes use the same component. Components without a loader use server state.

Loaders run on client navigation, not the initial SSR load unless the component
sets `static ssrLoader = true`. A failed loader logs a warning and falls back to
the server-provided state.

### Controlling State

Choose how state reaches each route component:

| Need | Mechanism | What happens |
|------|-----------|-------------|
| **Server provides state to authored code** | Authored route component | Fresh route state is applied when the component mounts |
| **Template-only soft navigation** | Omit sibling `.ts` / `.js`, load the framework once | The framework mounts the compiled route template |
| **I fetch my own data** | `static loader()` on component | Loader runs before the route commits and supplies route data |
| **Preserve local state** | `keep-alive` on route | Params/query attrs update while local state is preserved |
| **Preserve DOM + refresh data** | `keep-alive` + `static loader()` | DOM is preserved and loader data refreshes the component |

FAST route components expose scalar route state as kebab-case HTML
attributes, so `userName` becomes `user-name`. Strings, numbers, and booleans
are emitted. Complex values should be initialized from the rendered DOM or
another documented mechanism appropriate to the component. Keep the payload
flat when the component reads values through `@attr`.

```typescript
// Express example - the npm helper returns a complete JSON partial.
app.get('*', async (req, res) => {
  const state = await db.getPageState(req.path);
  const partialJson = protocol.renderPartial(
    state,
    'index.html',
    req.path,
    invHex,
  );
  res.type('json').send(partialJson);
});
```

Route components that only have `.html` and optional `.css` files do not need a
JavaScript loader or empty class. Create a custom element only when the route
component is interactive: event handlers, custom lifecycle code, imperative
methods, or JavaScript-owned state.

Initial SSR delivers component CSS only for the matched route chain. Styles for
inactive routes remain deferred until navigation activates those components,
including when an `<outlet />` is inside a Shadow component.

### Tagged Cache

The router caches partial responses and tags them with server-provided cache tags for precise invalidation. Enable caching at startup:

The cache is opt-in. Select it once at startup; a default `router.start()` does
not allocate it. Resource inventories do not cache application state.

```typescript
router.start({
  cache: navigationCache({
    staleTime: 30_000,   // ms before refetch (default: 0 = disabled)
    gcTime: 300_000,     // ms before eviction from memory (default: 5 min)
    maxEntries: 50,      // LRU cap (default: 50)
  }),
});
```

Declare cache tags on routes as HTML attributes. Placeholders like `{threadId}` reference route path parameters and are resolved at render time:

```html
<route path="/" component="mail-app">
  <route path="./" component="mail-view" keep-alive
         cache-tags="folders,counts">
    <route path="" component="inbox-page" exact
           cache-tags="inbox,counts" />
    <route path="email/:threadId" component="thread-page" exact
           cache-tags="thread:{threadId}" />
  </route>
</route>
```

Tag placeholders must name declared route parameters. For `/email/42`,
`thread:{threadId}` becomes `thread:42`. Revisit a cached path within
`staleTime` to reuse its response without another fetch.

<webui-blockquote appearance="tip" title="Preload + cache interaction" icon="💡">

With `preload: routePreload()`, hover fetches write to the same cache. Preloaded entries get a minimum 5-second freshness window even when `staleTime` is 0.

</webui-blockquote>

### Tag-Based Invalidation

Declare which tags a route invalidates after mutations:

```html
<route path="compose" component="compose-page" exact
       invalidates="inbox,sent,counts,drafts" />

<route path="email/:threadId/reply" component="reply-page" exact
       invalidates="thread:{threadId},inbox" />
```

The compiler validates the declaration. Choose tags that cover the data your
mutation changes; the router cannot infer your application's data dependencies.

Programmatic invalidation:

```typescript
router.invalidateTags(['inbox', 'thread:42']);  // evict by tag
router.invalidate('/email/42');                  // evict by path
router.invalidate();                             // evict everything
```

### Mutation Actions

The write counterpart to `static loader()`. Components define `static action()` to handle form submissions, and the router auto-invalidates the cache. Select it with `router.start({ actions: routeActions() })`, importing `routeActions` from `router.js`:

```typescript
import { WebUIElement } from '@microsoft/webui-framework';
import type { RouteActionContext, RouteActionResult } from '@microsoft/webui-delivery/router.js';

export class ComposePage extends WebUIElement {
  static async action({ formData, params, signal }: RouteActionContext): Promise<RouteActionResult> {
    await fetch('/api/send', { method: 'POST', body: formData, signal });
    return {
      invalidateTags: ['sent'],           // merged with route's invalidates attr
      state: { status: 'Message sent' },  // optimistic UI (optional)
    };
  }
}
```

With actions enabled, internal `<form method="post">` submissions call the
route's action. External action URLs and targets other than `_self` are left
alone. Returned `invalidateTags` combine with the route's `invalidates` tags;
optional `state` updates the component. Listen for
`webui:route:action-complete` to respond after completion.

### Pending UI

Show a loading component during slow navigations. The component is validated at build time - a typo causes a build error, not a runtime blank screen:

```html
<route path="inbox" component="inbox-page" exact
       pending="mail-skeleton" />
```

| Behavior | Description |
|----------|-------------|
| **Threshold** | Pending component appears when pre-commit navigation work exceeds 150ms - fast navigations never flash |
| **Mount** | Rendered in the parent route's outlet area |
| **Commit** | Removed atomically when settled route content commits, before `webui:route:navigated` |
| **Keep-alive** | Skipped - keep-alive routes activate instantly from the DOM |
| **Cached** | Skipped - cached navigations have no fetch delay |

Pending components are normal WebUI components - SSR'd, compiled, and part of the protocol. No special API needed.

### Error Boundaries

Show an error component when navigation fails. Like `pending`, the component is validated at build time:

```html
<route path="dashboard" component="dashboard-page" exact
       error="error-display" />
```

The error component receives details as state:

```typescript
export class ErrorDisplay extends WebUIElement {
  @observable error = '';    // "Navigation failed"
  @observable status = 0;    // HTTP status code (0 if network error)
  @observable path = '';     // the path that failed

  onRetry = () => router.navigate(this.path);
}
```

```html
<!-- error-display.html -->
<div class="error">
  <h2>Something went wrong</h2>
  <p>{{error}}</p>
  <button @click="{onRetry()}">Try again</button>
</div>
```

If no `error` attribute is declared on the route, the router falls back to its default behavior (`console.warn` + stale content preserved).

## How It Works

<a id="first-load-ssr"></a>
<a id="ssr-output"></a>
<a id="client-hydration"></a>

The server renders the matched components for the first request. Later browser
navigations fetch the destination from the server and replace only changed route
levels. Parent components remain mounted. Use the host rendering APIs below;
do not construct route bootstrap data yourself.

#### SSR Fresh / Loaders

By default, `router.start({ ssrFresh: true })` skips running route loaders on the initial SSR-bootstrapped navigation. The server-rendered state is considered authoritative, so there is no redundant client-side fetch on first load.

Components that need to run their loader even during SSR bootstrap can opt in:

```typescript
export class LiveDashboard extends WebUIElement {
  static ssrLoader = true; // loader runs even on SSR boot

  static async loader({ params, signal }: RouteLoaderContext) {
    const resp = await fetch(`/api/dashboard/${params.id}`, { signal });
    return resp.json();
  }
}
```

Loaders always run on subsequent client-side navigations regardless of the `ssrFresh` setting.

<a id="client-navigation"></a>

## The `WebUIRouter` API

### `router.start(config?)`

Starts an application-owned router. Call once after hydration completes.
The constructor takes no configuration.

```typescript
router.start({
  loaders: { ... },           // lazy-loading map (component tag -> async import)
  preload: routePreload(),    // speculative fetch on link hover
  actions: routeActions(),    // intercept POST forms and call static action()
  ssrFresh: true,             // skip initial loader replay (default: true)
  cache: navigationCache({    // tagged navigation cache
    staleTime: 30_000,        // ms before refetch (0 = disabled)
    gcTime: 300_000,          // ms before memory eviction
    maxEntries: 50,           // LRU cap
  }),
});
```

> **Base path:** The router automatically reads `<base href>` from the DOM. No `basePath` config needed — just set `<base href="/my-app/">` in your HTML.

### `router.navigate(path)`

```typescript
router.navigate('/users/42');
```

### `router.back()`

```typescript
router.back();
```

### `router.invalidateTags(tags)`

Evict all cache entries whose tags overlap with the given tags:

```typescript
router.invalidateTags(['inbox', 'thread:42']);
```

### `router.invalidate(path?)`

Evict cache entries by path, or all entries if no path is given:

```typescript
router.invalidate('/email/42');  // evict one entry
router.invalidate();             // evict everything
```

### `router.activeComponent`

The component tag of the currently active leaf route:

```typescript
console.log(router.activeComponent); // "user-detail"
```

### `router.activeParams`

The bound parameters of the current route:

```typescript
console.log(router.activeParams); // { id: "42" }
```

### `router.destroy()`

Tears down the router, removes event listeners, and clears the cache.

### `router.gc()`

Release cached component templates to free memory. Later navigation can fetch
them again when needed.

With the WebUI framework, import its optional cache adapter before runtime
startup or template reads, and use a freshly rebuilt protocol:

```typescript
import { startRuntime } from '@microsoft/webui-framework/runtime.js';
import '@microsoft/webui-framework/styles.js';
import '@microsoft/webui-framework/template-cache.js';
import './app-shell.js';

startRuntime();

router.gc();
```

Collection releases cache ownership while live components retain the ordinary
templates their conditions/repeats can still instantiate. Active pending/error
resources remain available. Inventory is reset conservatively; old in-flight
responses cannot restore the cleared inventory.
The optional navigation response cache is cleared as well, so its responses do
not continue retaining the same resources.

The operation does not destroy mounted components or unload JavaScript modules,
native custom-element definitions, or shared document styles. Garbage collection
is asynchronous. Calling `router.gc()` for WebUI templates without the adapter
now fails explicitly rather than reporting a release that can leave metadata
retained by component definitions.

<webui-blockquote appearance="tip" title="When to use this" icon="💡">

The template inventory already avoids duplicate downloads. Use `gc()` for a
long-lived app when measured template memory warrants the later refetch cost.

</webui-blockquote>

## Lazy Loading

Lazy-load route components so their JavaScript is only fetched on navigation:

```typescript
router.start({
  loaders: {
    'user-list': () => import('./pages/user-list.js'),
    'user-detail': () => import('./pages/user-detail.js'),
  },
});
```

- Import other authored component modules yourself; the router does not
  discover JavaScript files.
- Successful loader promises are reused. A rejected loader can be attempted
  again, but a failed native module import may remain cached by the browser.
- The router loads modules for the initial active SSR chain too. Rendered HTML
  is not a substitute for the component's event handlers.

## On-Demand Component Loading

Components like dialogs and overlays can be declared as routes but loaded
on demand instead of during navigation. Declare them in the route tree so
the build compiles them:

```html
<route path="/" component="app-shell">
  <route path="" component="home-page" exact />
  <route path="settings" component="settings-dialog" exact />
</route>
```

Then load dynamically before first use:

```typescript
await router.ensureLoaded('settings-dialog');
```

A component outside the active route stays deferred until it is requested.
`ensureLoaded` fetches its template before use. A direct navigation to
`/settings` instead renders it normally in the outlet.

With `@microsoft/webui-framework` loaded, `ensureLoaded()` also prepares the
component's styles. Stylesheet failures are reported; a component may render
unstyled rather than remain permanently hidden.

Configure a custom template endpoint:

```typescript
router.start({
  templateEndpoint: '/api/templates', // default: '/_webui/templates'
});
```

On the server, use the loaded protocol's `renderComponentTemplates` method:

```javascript
app.get('/_webui/templates', (req, res) => {
  const tags = (req.query.t ?? '').split(',').filter(Boolean);
  const inv = req.get('X-WebUI-Inventory') ?? '';
  res.type('json').send(protocol.renderComponentTemplates(tags, inv));
});
```

## Navigation Events

```typescript
window.addEventListener('webui:route:navigated', (event) => {
  const { component, params, query, path } = event.detail;
});

window.addEventListener('webui:route:action-complete', (event) => {
  const { component, invalidatedTags, path } = event.detail;
});
```

## Server Contract

The server handles two request types for each route:

### JSON Partial (client navigation)

Return the partial response produced by your host binding. It includes the
state, templates, styles, and route information the router needs; do not build
that response by hand.

**Request headers the router sends:**

| Header | Value | Purpose |
|--------|-------|---------|
| `Accept` | `application/x-ndjson, application/json` | Requests NDJSON streaming or JSON partial instead of HTML |
| `X-WebUI-Inventory` | Hex bitmask | Component template and style metadata the client already has - server skips re-sending it |

### Full HTML (initial load)

Without `Accept: application/json`, return the full SSR'd page. The handler
includes the route chain, template inventory, and CSS list needed for client
bootstrap. The CSS list contains the matched route chain, not inactive siblings.

### Partial Navigation

Use `Protocol::render_partial()` in Rust, `renderPartial()` in Node.js, or the
equivalent [host integration API](/guide/integrations/). Pass the current path,
client inventory header, and the state needed by the matched page.

For Rust hosts writing directly to a response writer,
`Protocol::prepare_partial()` returns a serializable response and `is_match()`
for status handling. Keep credentials and private data out of render state.

For repeated Rust requests, load one `Protocol`:

```rust
let protocol = Protocol::from_protobuf(&protocol_bytes)?;
let json = protocol.render_partial(
    state,
    "index.html",
    request_path,
    inventory_hex,
)?;
```

```csharp
// C#
string partialJson = protocol.RenderPartial(
    stateJson,
    entryId,
    requestPath,
    inventoryHex);
```

```javascript
// Node.js
const partialJson = protocol.renderPartial(
  state,
  entryId,
  requestPath,
  inventoryHex,
);
```

### Express Example

```javascript
import { Protocol } from '@microsoft/webui';
import { readFileSync } from 'node:fs';

const protocol = new Protocol(
  readFileSync('./dist/protocol.bin'),
  { plugin: 'webui' },
);

app.get('/users/:id', (req, res) => {
  const state = { name: getUser(req.params.id).name };

  if (req.accepts('json')) {
    const inv = req.get('X-WebUI-Inventory') ?? '';
    const partialJson = protocol.renderPartial(
      state,
      'index.html',
      req.path,
      inv,
    );
    res.type('json').send(partialJson);
  } else {
    const html = protocol.render(state, {
      entry: 'index.html',
      requestPath: req.path,
    });
    res.type('html').send(html);
  }
});
```

## Security

Route parameters (`:id`, `:name`, etc.) are extracted from URLs and injected into component state. They are automatically HTML-escaped when rendered with double braces (`{{param}}`), but **not** when rendered with triple braces (`{{{param}}}`).

> ⚠️ Never use triple braces (`{{{...}}}`) to render route parameters. An attacker could craft a URL like `/users/<script>alert(1)</script>` to inject arbitrary HTML.

Always validate route parameters on the server before including them in state.

## Route-Scoped State

For optimal performance, each route handler should return only the state that
its component template binds to - not the full application state.

### Anti-pattern: Full State for Every Route

```json
// Avoid returning unrelated state for every route
{
  "folders": [...],
  "threads": [...],
  "messages": [...],
  "settings": {...},
  "contacts": [...]
}
```

### Correct: Route-Scoped State

```json
// /inbox - what the inbox component needs
{ "threads": [...], "selectedFolder": "inbox" }

// /inbox/:id - what the detail component needs
{ "subject": "Q4 Review", "messages": [...] }

// /settings - settings data
{ "theme": "dark", "language": "en", "notifications": true }
```

Route-scoped state reduces one part of the navigation payload. Partials also
carry the route chain and any needed template or style data.

## Styling Route Outlets

`<webui-route>` elements rendered by `<outlet />` are bare custom elements
with `display: inline` by default. If the outlet's parent uses flexbox or
grid layout, you need to style the route element:

```css
/* In the parent component's CSS */
.content-area > webui-route {
  display: flex;
  flex-direction: column;
  flex: 1;
}
```

Hidden routes use inline `display: none`. Do not override that with
`!important`. You can also restrict layout rules to visible routes:

```css
.content-area > webui-route:not([style*="display:none"]) {
  display: flex;
  flex-direction: column;
  flex: 1;
}
```

## Full Example

```html
<!-- index.html -->
<body>
  <route path="/" component="app-shell">
    <route path="" component="home-page" exact />
    <route path="contacts" component="contacts-page">
      <route path="add" component="contact-form" exact />
      <route path=":id" component="contact-detail" exact />
      <route path=":id/edit" component="contact-form" exact />
    </route>
  </route>
  <script type="module" src="/index.js"></script>
</body>
```

```html
<!-- app-shell.html -->
<header><nav-bar></nav-bar></header>
<main><outlet /></main>
```

```html
<!-- contacts-page.html -->
<h2>Contacts</h2>
<div class="list">...</div>
<outlet />
```

```typescript
// index.ts
import { startRuntime } from '@microsoft/webui-framework/runtime.js';
import '@microsoft/webui-framework/styles.js';
import { WebUIRouter } from '@microsoft/webui-delivery/router.js';
import './app-shell.js';

const router = new WebUIRouter();

window.addEventListener('webui:hydration-complete', () => {
  router.start({
    loaders: {
      'home-page': () => import('./pages/home-page.js'),
      'contacts-page': () => import('./pages/contacts-page.js'),
      'contact-form': () => import('./pages/contact-form.js'),
      'contact-detail': () => import('./pages/contact-detail.js'),
    },
  });
});

startRuntime();
```
