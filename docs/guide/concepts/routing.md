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
- **`mail-view` (keep-alive):** Hidden on deactivation, shown instantly on return. The folder pane, email list, and all local state survive the round trip. Route param and query param attributes are updated, but `setState()` is **not called** — your component's `@observable` properties are preserved.
- **`settings-page` (no keep-alive):** Destroyed on deactivation, recreated fresh on each visit.

| Behavior | With `keep-alive` | Without |
|----------|-------------------|---------|
| Deactivate | `display: none` (stays in DOM) | `display: none` (stays in DOM) |
| Reactivate | Reuses existing component — params updated, state preserved | Destroys old, creates new component |
| Local state | ✅ Preserved (scroll, input, timers, observables) | Lost |
| Server state | **Skipped** — use a [loader](#route-loaders) to refresh | Applied on mount via `setState()` |

<webui-blockquote appearance="tip" title="When to use" icon="💡">

Use on routes with expensive UI (lists, grids, trees) that users switch between frequently. Leaf routes with simple data-driven content rarely benefit — they're cheap to recreate.

</webui-blockquote>

<webui-blockquote appearance="tip" title="Refreshing data on reactivation" icon="💡">

If a keep-alive component needs fresh data when reactivated, define a static `loader()` method. The router calls it on every navigation (including reactivation) and applies the result via `setState()`.

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
- Components without a `loader()` use server state as before — fully backwards compatible

### Controlling State

The router provides four mechanisms for controlling how state flows to your components:

| Need | Mechanism | What happens |
|------|-----------|-------------|
| **Server provides all state** | Default (no changes) | `setState(state)` on every navigation |
| **I fetch my own data** | `static loader()` on component | Loader runs pre-commit, result passed to `setState()` |
| **Preserve local state** | `keep-alive` on route | Params/query attrs updated, `setState()` skipped |
| **Preserve DOM + refresh data** | `keep-alive` + `static loader()` | DOM preserved, loader result applied via `setState()` |

```typescript
// Express example — render_partial returns chain + templates (no state).
// Caller adds state to the response.
app.get('*', async (req, res) => {
  const state = await db.getPageState(req.path);
  const partial = handler.renderPartial(protocol, index, req.path, invHex);
  partial.state = state;
  res.json(partial);
});
```

### Tagged Cache

The router caches partial responses and tags them with server-provided cache tags for precise invalidation. Enable caching at startup:

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

The write counterpart to `static loader()`. Components define `static action()` to handle form submissions, and the router auto-invalidates the cache:

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
| **5. Update** | Applies optimistic `result.state` via `setState()` if provided |
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
5. JavaScript loads, hydration runs, router starts and reads `window.__webui`

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

Build-time attributes like `query`, `keep-alive`, `cache-tags`, and `invalidates` are **not** emitted as DOM attributes on `<webui-route>` elements. They are compiled into the binary protocol and delivered to the client via `window.__webui.chain` JSON data. The `<route>` source attributes remain valid and unchanged - the compiler just delivers them through JSON instead of the DOM.

The server also emits a `window.__webui` script containing the SSR chain, template inventory, and CSS metadata. This replaces the previous `<meta name="webui-inventory">` tag (which is still supported as a fallback for older servers).

```html
<script>window.__webui = {
  chain: [
    { "component": "app-shell", "path": "/", "keepAlive": false },
    { "component": "topic-page", "path": "topics/:topicId", "params": { "topicId": "react" }, "exact": true }
  ],
  inventory: "04000400...",
  nonce: "abc123",
  css: ["/styles/main.css"],
  styles: ["app-shell", "topic-page"]
};</script>
```

#### Client Hydration

At startup, the router reads `window.__webui` instead of walking the DOM:

1. **Chain**: The SSR chain is provided as JSON in `window.__webui.chain`, eliminating DOM walking and URLPattern usage
2. **Element binding**: `data-ri` attributes on `<webui-route>` elements enable O(1) lookup by chain index - no component-name matching needed
3. **Inventory**: `window.__webui.inventory` provides the template bitmask (falls back to `<meta name="webui-inventory">` for older servers)
4. **CSS/Styles**: `window.__webui.css` and `window.__webui.styles` track injected assets

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

Release all cached component templates to free memory. Removes all entries from `window.__webui.templates` and clears their inventory bits so the server will re-send them on the next navigation that needs them.

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

The template is **not** sent during initial SSR or partial navigation —
only when explicitly requested via `ensureLoaded`. If a user navigates
directly to `/settings`, the component renders normally in the outlet.

Configure a custom template endpoint:

```typescript
Router.start({
  templateEndpoint: '/api/templates', // default: '/_webui/templates'
});
```

On the server, handle the template endpoint with `renderComponentTemplates`:

```javascript
import { renderComponentTemplates } from '@microsoft/webui';

app.get('/_webui/templates', (req, res) => {
  const tags = (req.query.t ?? '').split(',').filter(Boolean);
  const inv = req.get('X-WebUI-Inventory') ?? '';
  res.type('json').send(renderComponentTemplates(protocol, tags, inv));
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
  "templateStyles": ["<style type=\"module\" specifier=\"user-detail\">...</style>"],
  "templates": ["(function(){var w=window.__webui.templates||...})();"],
  "inventory": "04000400...",
  "path": "/users/42",
  "chain": [
    { "component": "app-shell", "path": "/" },
    {
      "component": "user-detail", "path": "users/:id",
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
| `state` | Application state (added by the caller, not by `render_partial`) |
| `templateStyles` | Module CSS definition tags (empty for Link/Style modes) |
| `templates` | Client template payloads filtered by inventory bitmask |
| `inventory` | Updated hex bitmask of loaded templates |
| `path` | The matched request path |
| `chain` | Matched route chain - one entry per nesting level |
| `cacheTags` | Resolved cache tags from the full chain (union of all levels) |
| `cacheControl` | Optional per-response cache overrides |

Each `chain` entry can include: `component`, `path`, `params`, `exact`, `keepAlive`, `allowedQuery`, `pendingComponent`, `errorComponent`, and `invalidates`.

**Request headers the router sends:**

| Header | Value | Purpose |
|--------|-------|---------|
| `Accept` | `application/x-ndjson, application/json` | Requests NDJSON streaming or JSON partial instead of HTML |
| `X-WebUI-Inventory` | Hex bitmask | Templates the client already has — server skips re-sending them |

### Full HTML (initial load)

Without `Accept: application/json`, return the full SSR'd page. The handler emits a `window.__webui` script in `<head>` containing the SSR chain, template inventory, and CSS metadata so the client router can bootstrap without DOM walking.

### Partial Navigation

The partial response format is unchanged. Use `render_partial()` (Rust) or `webui_render_partial()` (FFI) to get the partial response - chain, templateStyles, templates, inventory, path, and cacheTags. The caller adds application state to the result.

`render_partial()` now requires a `ProtocolIndex` parameter - a pre-computed index that caches expensive lookups (component bit-index maps, compiled route templates, and component closures). Build it once per protocol at startup and reuse it across requests:

```rust
// Rust - build the index once, reuse across requests
let mut index = ProtocolIndex::new(&protocol);

let mut partial = route_handler::render_partial(&protocol, &entry, &path, &inventory_hex, &mut index);
// Caller adds state to the response
if let Some(obj) = partial.as_object_mut() {
    obj.insert("state".into(), state);
}
```

```csharp
// C#
string partialJson = handler.RenderPartial(protocol, index, entryId, requestPath, inventoryHex);
// Caller merges state into the JSON before sending
```

```javascript
// Node.js
const partialJson = webui.renderPartial(protocol, index, entryId, requestPath, inventoryHex);
// Caller adds state before sending
```

### Express Example

```javascript
// Build index once at startup
const index = webui.createIndex(protocol);

app.get('/users/:id', (req, res) => {
  const state = { name: getUser(req.params.id).name };

  if (req.accepts('json')) {
    // renderPartial() returns chain + templates; caller adds state
    const inv = req.get('X-WebUI-Inventory') ?? '';
    const partial = JSON.parse(webui.renderPartial(protocol, index, 'index.html', req.path, inv));
    partial.state = state;
    res.type('json').send(JSON.stringify(partial));
  } else {
    res.type('html').send(handler.render(protocol, state, 'index.html', req.path));
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

// Shell component — eagerly loaded (registers custom element, triggers hydration)
import './app-shell.js';
```
