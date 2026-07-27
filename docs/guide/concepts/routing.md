# Routing

WebUI routes are **declared in HTML and compiled at build time**. The Rust compiler parses your `<route>` tree, validates every attribute, and bakes it all into the binary protocol. Cache tags, invalidation graphs, pending states, error boundaries - everything is known before a single request is served.

This means:
- **Zero runtime JavaScript for routing policy.** Cache semantics, invalidation rules, and loading states are HTML attributes - not framework configuration objects.
- **Build-time validation.** A typo in `pending="loadnig-skeleton"` is a compile error, not a blank screen in production.
- **Server and client both know the full graph.** The server resolves cache tags with real param values. The client invalidates by tag after mutations. Neither needs runtime discovery.

At its simplest, routing is three lines of HTML and one line of TypeScript. At its most advanced, it's declarative tagged caching with compiler-enforced invalidation graphs - and everything in between uses the same `<route>` element.

## Installation

```bash
npm install @microsoft/webui-router
```

Only needed when your app has client-side navigation. Server-only apps with full page loads don't need it.

## Quick Start

**1. Declare routes in `index.html`:**

**1. Add `<base href="/">` in your `<head>`:**

```html
<head>
  <meta charset="utf-8">
  <base href="/">
</head>
```

All WebUI apps with routes **must** include `<base href="/">`. Without it, relative asset paths (CSS, JS) break on nested routes — the browser resolves `app.css` against `/users/123/` → `/users/app.css` instead of `/app.css`.

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

**2. Use `<outlet />` in your shell component:**

```html
<!-- app-shell.html -->
<nav><a href="/">Home</a> <a href="/users">Users</a></nav>
<main><outlet /></main>
```

**3. Start the router:**

```typescript
import { Router } from '@microsoft/webui-router';

Router.start();
```

The server SSRs the matched route on first load. The router handles clicks on `<a>` tags for subsequent navigations - no full page reloads.

Route segments may contain literal dots, such as `/docs/v2.1` or
`/users/john.doe`. In `webui serve`, a missed asset lookup falls back to route
rendering only for requests that explicitly accept `text/html`,
`application/xhtml+xml`, or `application/json`. `q=0` disables that media type,
while a malformed or out-of-range `q` value falls back to `q=1.0`; when HTML and
JSON are both acceptable, the higher `q` wins and exact ties prefer JSON. Missing
or wildcard-only `Accept` headers return 404, as
do JS, CSS, image, and other
non-HTML/non-JSON asset requests.

The router never imports framework code. Authored route components use their
registered classes. When the application also loads
`@microsoft/webui-framework`, HTML-only route templates can mount during soft
navigation without empty TypeScript modules. The router falls back to a full
document request only when no runtime registers the destination tag.

When the View Transitions API is available, client-side route commits
use `document.startViewTransition()`. While active, the router installs a
nonce-bearing `@view-transition { navigation: none; }` override. This disables
automatic cross-document transitions because they conflict with intercepted
routes that may need the document fallback. `Router.destroy()` removes the
override.

## Nested Routes

Routes nest to any depth. Each parent component uses `<outlet />` where its child route renders:

```html
<!-- index.html -->
<route path="/" component="app-shell">
  <route path="" component="dashboard" exact />
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
    <route path="profile" component="profile" exact />
    <route path="billing" component="billing" exact />
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

Opt-in speculative fetching — the router prefetches route data when the user hovers an internal link, so navigation on click is instant:

```ts
Router.start({ preload: true });
```

How it works:
- On mouse hover over an internal `<a>`, the router speculatively calls `fetchPartial()` for that path
- Results are stored in the [tagged cache](#tagged-cache) with a 5-second minimum freshness
- On click, the cached result is used immediately - no network wait
- If the user hovers a different link, the previous preload is cancelled and a new one starts

Only mouse pointers trigger preload — touch taps fire simultaneously with the click event, making speculative fetching pointless.

Preload is an optional runtime tier. Apps that do not enable it do not load the
preload listener or navigation cache implementation.

### Route Loaders

Define a static `loader()` method on a component class to fetch data from a custom source instead of using server-provided state:

```typescript
import { WebUIElement } from '@microsoft/webui-framework';
import type { RouteLoaderContext } from '@microsoft/webui-router';

export class LiveDashboard extends WebUIElement {
  static async loader({ params, signal }: RouteLoaderContext) {
    const resp = await fetch(`/api/dashboard/${params.id}`, { signal });
    return resp.json();
  }
}
```

How it works:
- The router checks each route component's constructor for a static `loader()` method
- Loaders run **before** the view transition — results are ready for synchronous DOM commit
- The loader receives route `params`, `query`, and an `AbortSignal` tied to the navigation
- If a loader fails, the router falls back to server-provided `data.state` with a console warning
- Loaders run on both SSR bootstrap and SPA navigations for consistency
- Components without a `loader()` use server state

### Controlling State

The router provides four mechanisms for controlling how state flows to your components:

| Need | Mechanism | What happens |
|------|-----------|-------------|
| **Server provides state to authored code** | Authored route component | Fresh route state is applied when the component mounts |
| **Template-only soft navigation** | Omit sibling `.ts` / `.js`, load the framework once | The framework mounts the compiled route template |
| **I fetch my own data** | `static loader()` on component | Loader runs before the route commits and supplies route data |
| **Preserve local state** | `keep-alive` on route | Params/query attrs update while local state is preserved |
| **Preserve DOM + refresh data** | `keep-alive` + `static loader()` | DOM is preserved and loader data refreshes the component |

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

### Tagged Cache

The router caches partial responses and tags them with server-provided cache tags for precise invalidation. Enable caching at startup:

The cache is opt-in. The default `Router.start()` path does not load the cache
module; enabling `cache` or `preload` loads it on demand.

```typescript
Router.start({
  cache: {
    staleTime: 30_000,   // ms before refetch (default: 0 = disabled)
    gcTime: 300_000,     // ms before eviction from memory (default: 5 min)
    maxEntries: 50,      // LRU cap (default: 50)
  },
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

| Behavior | Description |
|----------|-------------|
| **Build time** | The Rust compiler validates `{param}` placeholders match actual route params |
| **Render time** | The handler resolves `thread:{threadId}` to `thread:42` using matched params |
| **Response** | Resolved tags are included in the `cacheTags` array of the JSON partial |
| **Client** | The router stores the response keyed by path, tagged with resolved values |
| **Revisit** | Within `staleTime`, the cached response is used instantly - no network fetch |
| **Server override** | The server can include `cacheControl: { staleTime: 60000 }` to override per-response |

<webui-blockquote appearance="tip" title="Preload + cache interaction" icon="💡">

When `preload: true` is enabled, hover fetches write to the same cache. Preloaded entries get a minimum 5-second freshness window even when `staleTime` is 0 (disabled).

</webui-blockquote>

### Tag-Based Invalidation

Declare which tags a route invalidates after mutations:

```html
<route path="compose" component="compose-page" exact
       invalidates="inbox,sent,counts,drafts" />

<route path="email/:threadId/reply" component="reply-page" exact
       invalidates="thread:{threadId},inbox" />
```

The compiler builds the full invalidation graph at build time. Developers declare intent in HTML - the framework ensures correctness.

Programmatic invalidation:

```typescript
Router.invalidateTags(['inbox', 'thread:42']);  // evict by tag
Router.invalidate('/email/42');                  // evict by path
Router.invalidate();                             // evict everything
```

### Mutation Actions

The write counterpart to `static loader()`. Components define `static action()` to handle form submissions, and the router auto-invalidates the cache. Enable this optional runtime with `Router.start({ actions: true })`:

```typescript
import { WebUIElement } from '@microsoft/webui-framework';
import type { RouteActionContext, RouteActionResult } from '@microsoft/webui-router';

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

The router intercepts `<form method="post">` submissions via a delegated listener:

| Step | What happens |
|------|-------------|
| **1. Intercept** | Walks `composedPath()` to find the form and nearest `<webui-route>` (shadow DOM safe) |
| **2. Guard** | Skips forms with external `action` URLs or `target` other than `_self` |
| **3. Call** | Invokes `static action({ formData, params, signal })` on the component class |
| **4. Invalidate** | Merges the action's returned tags with the route's build-time `invalidates` attribute |
| **5. Update** | Applies optimistic `result.state` if provided |
| **6. Event** | Dispatches `webui:route:action-complete` on `window` |

### Pending UI

Show a loading component during slow navigations. The component is validated at build time - a typo causes a build error, not a runtime blank screen:

```html
<route path="inbox" component="inbox-page" exact
       pending="mail-skeleton" />
```

| Behavior | Description |
|----------|-------------|
| **Threshold** | Pending component appears after 150ms - fast navigations never flash |
| **Mount** | Rendered in the parent route's outlet area |
| **Replace** | Real content replaces the skeleton when the fetch completes |
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

  onRetry = () => Router.navigate(this.path);
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

### First Load (SSR)

1. Browser requests `/sections/1/topics/react`
2. Server matches the full route chain: `app-shell - section-page - topic-page`
3. Renders all matched components nested at their outlets
4. Browser displays fully rendered HTML - no JavaScript needed yet
5. JavaScript loads, hydration runs, and the router starts from the SSR bootstrap data

#### SSR Output

The server renders `<webui-route>` elements with these DOM attributes:

| Attribute | Purpose |
|-----------|---------|
| `path` | The route's path pattern |
| `component` | The component tag name |
| `active` | Present on matched routes |
| `exact` | Present on leaf routes |
| `pending` | Pending component tag (if declared) |
| `error` | Error component tag (if declared) |
| `data-ri` | Route index for O(1) element binding during hydration |

Build-time attributes like `query`, `keep-alive`, `cache-tags`, and `invalidates` are **not** emitted as DOM attributes on `<webui-route>` elements. The `<route>` source attributes remain valid and unchanged, while the rendered route elements expose only the runtime attributes needed for navigation.

The server also emits inert route bootstrap data so the client router can start without walking the DOM.

```html
<script type="application/json" id="webui-data">
{
  "chain": [
    { "component": "app-shell", "path": "/", "keepAlive": false },
    { "component": "topic-page", "path": "topics/:topicId", "params": { "topicId": "react" }, "exact": true }
  ],
  "inventory": "04000400...",
  "nonce": "abc123",
  "css": ["/styles/main.css"],
  "styles": ["app-shell", "topic-page"],
  "state": { "title": "Topic" }
}
</script>
```

#### Client Hydration

At startup, the router uses the SSR route chain, route indexes, template
inventory, and style list emitted by the server. That keeps hydration direct and
avoids DOM walking or route-pattern recomputation on first load.

The `state` field is **projected** to the hydratable surface rather than
carrying your entire application state. At build time WebUI records which
properties each component actually hydrates — its template state roots plus any
`@observable`/`@attr` fields — and at render time the server emits only those
keys, dropping everything else (including server-only data). This keeps the
bootstrap block small even when the underlying render state is large; the client
hydrates exactly as before, since it only ever reads the properties a component
observes.

#### SSR Fresh / Loaders

By default, `Router.start({ ssrFresh: true })` skips running route loaders on the initial SSR-bootstrapped navigation. The server-rendered state is considered authoritative, so there is no redundant client-side fetch on first load.

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

### Client Navigation

1. User clicks a link to `/sections/1/topics/css`
2. Router intercepts via the [Navigation API](https://developer.mozilla.org/en-US/docs/Web/API/Navigation_API)
3. Fetches JSON partial from server with `Accept: application/json`
4. Server returns the matched route chain - the client does **not** perform route matching
5. Compares old chain with new - finds first changed level
6. Mounts only the changed component - parents stay mounted
7. No full page reload

## The `Router` API

### `Router.start(config?)`

Starts the router. Call after hydration completes.

```typescript
Router.start({
  loaders: { ... },           // lazy-loading map (component tag -> async import)
  preload: true,              // speculative fetch on link hover
  actions: true,              // intercept POST forms and call static action()
  ssrFresh: true,             // skip initial loader replay (default: true)
  cache: {                    // tagged navigation cache
    staleTime: 30_000,        // ms before refetch (0 = disabled)
    gcTime: 300_000,          // ms before memory eviction
    maxEntries: 50,           // LRU cap
  },
});
```

> **Base path:** The router automatically reads `<base href>` from the DOM. No `basePath` config needed — just set `<base href="/my-app/">` in your HTML.

### `Router.navigate(path)`

```typescript
Router.navigate('/users/42');
```

### `Router.back()`

```typescript
Router.back();
```

### `Router.invalidateTags(tags)`

Evict all cache entries whose tags overlap with the given tags:

```typescript
Router.invalidateTags(['inbox', 'thread:42']);
```

### `Router.invalidate(path?)`

Evict cache entries by path, or all entries if no path is given:

```typescript
Router.invalidate('/email/42');  // evict one entry
Router.invalidate();             // evict everything
```

### `Router.activeComponent`

The component tag of the currently active leaf route:

```typescript
console.log(Router.activeComponent); // "user-detail"
```

### `Router.activeParams`

The bound parameters of the current route:

```typescript
console.log(Router.activeParams); // { id: "42" }
```

### `Router.destroy()`

Tears down the router, removes event listeners, and clears the cache.

### `Router.gc()`

Release cached component templates to free memory. The router clears matching
inventory bits so the server will re-send templates on the next navigation that
needs them.

```typescript
Router.gc();
```

<webui-blockquote appearance="tip" title="When to use this" icon="💡">

Most apps don't need this - the number of unique component templates is bounded by the route tree (typically 10-30). The server's inventory system already prevents duplicate downloads. Use `gc()` in long-lived SPAs with many routes where memory pressure is a concern.

</webui-blockquote>

## Lazy Loading

Lazy-load route components so their JavaScript is only fetched on navigation:

```typescript
Router.start({
  loaders: {
    'user-list': () => import('./pages/user-list.js'),
    'user-detail': () => import('./pages/user-detail.js'),
  },
});
```

- Components **not in `loaders`** are eagerly loaded
- Each loader runs **at most once** - cached after first call
- On SSR'd initial load, the lazy loader is skipped (content already rendered)

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
await Router.ensureLoaded('settings-dialog');
```

The template is **not** sent during initial SSR or partial navigation. It is
loaded only when explicitly requested via `ensureLoaded`. If a user navigates
directly to `/settings`, the component renders normally in the outlet.

Configure a custom template endpoint:

```typescript
Router.start({
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

When `Accept: application/json` or `application/x-ndjson`:

```json
{
  "state": { "name": "Alice", "email": "alice@example.com" },
  "templateStyles": ["<script type=\"importmap\">{\"imports\":{\"user-detail\":\"data:text/css,...\"}}</script>"],
  "templates": {
    "user-detail": { "h": "<section></section>" }
  },
  "templateFunctions": {
    "user-detail": "[function(v,s){return !!v(\"ready\",s)}]"
  },
  "inventory": "04000400...",
  "path": "/users/42",
  "chain": [
    { "component": "app-shell", "client": true, "path": "/" },
    {
      "component": "user-detail", "client": true, "path": "users/:id",
      "params": { "id": "42" }, "exact": true, "keepAlive": true,
      "pendingComponent": "loading-skeleton",
      "errorComponent": "error-page",
      "invalidates": ["user:42", "users"]
    }
  ],
  "cacheTags": ["user:42", "users"],
  "cacheControl": { "staleTime": 60000 }
}
```

| Field | Description |
|-------|-------------|
| `state` | Active-route navigation state for reachable authored and scriptless components. `Protocol::render_partial` and all host bindings include it |
| `templateStyles` | Module CSS definition tags (empty for Link/Style modes) |
| `templates` | Client template payloads filtered by inventory bitmask |
| `inventory` | Updated hex bitmask of loaded templates |
| `path` | The matched request path |
| `chain` | Matched route chain - one entry per nesting level |
| `cacheTags` | Resolved cache tags from the full chain (union of all levels) |
| `cacheControl` | Optional per-response cache overrides |

Each `chain` entry includes `component` and `path`. It can also include
`params`, `exact`, `keepAlive`, `allowedQuery`, `pendingComponent`,
`errorComponent`, and `invalidates`. Component capability is determined by
custom-element registration, not by a server `client` flag.

**Request headers the router sends:**

| Header | Value | Purpose |
|--------|-------|---------|
| `Accept` | `application/x-ndjson, application/json` | Requests NDJSON streaming or JSON partial instead of HTML |
| `X-WebUI-Inventory` | Hex bitmask | Templates the client already has — server skips re-sending them |

### Full HTML (initial load)

Without `Accept: application/json`, return the full SSR'd page. The handler
includes the route chain, template inventory, and CSS list needed for client
bootstrap.

### Partial Navigation

Rust `Protocol::render_partial()` and every host binding return the complete
response, including the state needed by active-route components. Raw state
input is validated in full while unneeded values are skipped without
constructing a duplicate JSON tree.

For repeated Rust requests, load one `Protocol`:

```rust
let protocol = Protocol::from_protobuf(&protocol_bytes)?;
let json = protocol.render_partial(
    state_json,
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
// ❌ Returns everything for every route - 240 KB per navigation
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
// ✅ /inbox - only what the inbox component needs - 15 KB
{ "threads": [...], "selectedFolder": "inbox" }

// ✅ /inbox/:id - only what the detail component needs - 5 KB  
{ "subject": "Q4 Review", "messages": [...] }

// ✅ /settings - only settings data - 2 KB
{ "theme": "dark", "language": "en", "notifications": true }
```

Route-scoped state keeps JSON payloads small during client-side navigation,
where only the `state` field of the JSON partial is transferred.

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

Hidden routes use `style="display:none"` inline. If your CSS sets
`display: flex`, add specificity to avoid showing hidden routes:

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
import { Router } from '@microsoft/webui-router';

window.addEventListener('webui:hydration-complete', () => {
  Router.start({
    loaders: {
      'home-page': () => import('./pages/home-page.js'),
      'contacts-page': () => import('./pages/contacts-page.js'),
      'contact-form': () => import('./pages/contact-form.js'),
      'contact-detail': () => import('./pages/contact-detail.js'),
    },
  });
});

// Shell component - eagerly loaded
import './app-shell.js';
```
