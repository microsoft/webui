# @microsoft/webui-router

Build-time compiled router for [WebUI](https://github.com/microsoft/webui) apps. Routes, cache tags, invalidation graphs, pending states, and error boundaries are declared as HTML attributes, validated by the Rust compiler, and baked into the binary protocol — zero runtime JavaScript for routing policy.

Uses the [Navigation API](https://developer.mozilla.org/en-US/docs/Web/API/Navigation_API) for client-side transitions. The server provides the matched route chain; the client does not perform route matching.

> 📖 **Full documentation at [microsoft.github.io/webui](https://microsoft.github.io/webui)** — see the [Routing Guide](https://microsoft.github.io/webui/guide/concepts/routing) for setup and usage.

## How It Works

1. **Server renders the full page** - the matched route chain is SSR'd with declarative shadow roots. The page is interactive before JavaScript loads.
2. **Hydration completes** - WebUI Framework hydrates shell components.
3. **Router starts** - reads the SSR chain and metadata from `window.__webui` (JSON bootstrap), then intercepts link clicks via the Navigation API. Falls back to DOM-based discovery for older servers.
4. **Client-side navigation** - fetches a JSON partial from the server, which includes the matched route chain. The client diffs old vs new chain and mounts only the changed component. Parent components stay mounted.

No full page reloads. The shell stays in place. Only route content changes.

## Installation

```bash
npm install @microsoft/webui-router
```

## Quick Start

**1. Add `<base href="/">` in your `<head>`:**

```html
<head>
  <meta charset="utf-8">
  <base href="/">
</head>
```

All WebUI apps with routes **must** include `<base href="/">`. Without it, relative asset paths (CSS, JS) break on nested routes — the browser resolves `app.css` against `/contacts/123/` → `/contacts/app.css` instead of `/app.css`.

For sub-path deployments, set the base to the sub-path: `<base href="/my-app/">`.

**2. Declare nested routes in `index.html`:**

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

Child routes use **relative paths** (no leading `/`). The nesting is the route tree.

**2. Use `<outlet />` in parent components:**

```html
<!-- app-shell.html -->
<nav>
  <a href="/">Home</a>
  <a href="/users">Users</a>
</nav>
<main><outlet /></main>
<footer>© 2026</footer>
```

`<outlet />` marks where child route content renders. The nav and footer persist across navigations.

**3. Start the router after hydration:**

```typescript
import { Router } from '@microsoft/webui-router';

import './app-shell.js';

window.addEventListener('webui:hydration-complete', () => {
  Router.start({
    loaders: {
      'home-page': () => import('./pages/home-page.js'),
      'user-list': () => import('./pages/user-list.js'),
      'user-detail': () => import('./pages/user-detail.js'),
    },
  });
});
```

Components in `loaders` are lazy-loaded on first navigation. Components not listed are assumed eagerly loaded.

## Nested Routes

Routes nest to any depth. Each parent uses `<outlet />` for child content:

```html
<route path="/" component="app-shell">
  <route path="" component="dashboard" exact />
  <route path="settings" component="settings-page">
    <route path="profile" component="profile-page" exact />
    <route path="billing" component="billing-page" exact />
  </route>
</route>
```

Navigating from `/settings/profile` to `/settings/billing` only remounts the billing component — `app-shell` and `settings-page` stay mounted with their state preserved.

### The `exact` Attribute

- **Leaf routes** (no children): add `exact`
- **Parent routes** (have `<outlet />`): omit `exact`

Without `exact`, a route matches any URL that starts with its path — which is what parent routes need.

### The `keep-alive` Attribute

Preserve a component across navigations instead of destroying and recreating it:

```html
<route path="./" component="mail-view" keep-alive>
  <route path="" component="inbox-page" exact />
</route>
<route path="calendar" component="calendar-page" exact keep-alive />
```

When the user navigates away from a `keep-alive` route and returns, the existing component is reused — its DOM and local state (scroll position, input values, timers) survive the round trip.

**State is preserved by default.** The router only updates route param and query param attributes on reactivation — it does NOT call `setState()` with server data. This means your component's `@observable` properties, scroll position, form inputs, and any client-computed state all survive.

To refresh data on reactivation, define a [route loader](#route-loaders):

```typescript
export class MailView extends WebUIElement {
  @observable messages = [];

  static async loader({ signal }: RouteLoaderContext) {
    const resp = await fetch('/api/messages', { signal });
    return { messages: await resp.json() };
  }
}
```

### Route Loaders

Define a static `loader()` on a component to fetch data from a custom source instead of using server-provided state:

```typescript
import type { RouteLoaderContext } from '@microsoft/webui-router';

export class LiveDashboard extends WebUIElement {
  @observable source = '';
  @observable metrics = {};

  static async loader({ params, query, signal }: RouteLoaderContext) {
    const resp = await fetch(`/api/dashboard/${params.id}`, { signal });
    return { source: 'client', metrics: await resp.json() };
  }
}
```

**How it works:**
- The router checks each route component's constructor for a static `loader()` method
- Loaders run **before** the view transition — results are ready for synchronous DOM commit
- The loader receives route `params`, parsed `query`, and an `AbortSignal` tied to the navigation
- If a loader fails, the router falls back to server-provided state with a console warning
- Loaders run on both SSR bootstrap and SPA navigations for consistency

**When to use loaders:**
- WebSocket-driven dashboards that manage their own data stream
- Components that fetch from a different API than the SSR server
- Keep-alive components that need fresh data on reactivation
- Any component that wants full control over its state source

### Base Path (Sub-Path Deployment)

Every WebUI app with routes needs `<base href="/">` in its `<head>` (see Quick Start above). This ensures relative asset paths resolve correctly on nested routes.

For sub-path deployments (e.g., `https://example.com/commerce/`), change it to the sub-path:

```html
<head>
  <base href="/commerce/">
</head>
```

The `<base>` tag is a core web platform feature. It makes the browser resolve all relative URLs (`<a href>`, `<link href>`, `fetch()`) against the base path. The router detects it at startup and uses it to strip/prepend the prefix on navigation URLs.

When using `webui serve --base-path /commerce/`, the `<base>` tag is emitted automatically.

### Preload on Hover

Opt-in speculative fetching for instant click navigation:

```typescript
Router.start({ preload: true });
```

When enabled, the router prefetches JSON partials (templates, CSS, state) when the user hovers over internal links. On click, the cached result is used immediately. Preloaded entries are stored in the [tagged cache](#tagged-cache) with a 5-second minimum freshness. Only mouse pointers trigger preload.

### Tagged Cache

Cache partial responses with server-provided tags for precise invalidation:

```typescript
Router.start({
  cache: {
    staleTime: 30_000,   // ms before refetch (default: 0 = disabled)
    gcTime: 300_000,     // ms before eviction from memory (default: 5 min)
    maxEntries: 50,      // LRU cap (default: 50)
  },
});
```

Declare cache tags on routes as HTML attributes:

```html
<route path="email/:threadId" component="thread-page" exact
       cache-tags="thread:{threadId},inbox" />
```

The `{threadId}` placeholder is resolved at render time by the Rust handler. The server includes resolved `cacheTags` in the JSON partial. On revisit within `staleTime`, the cached response is used instantly.

### Tag-Based Invalidation

Declare which tags a route invalidates after mutations:

```html
<route path="compose" component="compose-page" exact
       invalidates="inbox,sent,counts,drafts" />
```

Programmatic invalidation:

```typescript
Router.invalidateTags(['inbox', 'thread:42']);  // evict by tag
Router.invalidate('/email/42');                  // evict by path
Router.invalidate();                             // evict everything
```

### Mutation Actions

The write counterpart to `static loader()`. The router intercepts `<form method="post">` and auto-invalidates the cache:

```typescript
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

The action's returned tags are merged with the route's build-time `invalidates` attribute. Only same-origin forms are intercepted.

### Pending UI

Show a loading component during slow navigations (>150ms):

```html
<route path="inbox" component="inbox-page" exact pending="mail-skeleton" />
```

The `pending` component is validated at build time. Keep-alive and cached routes skip pending.

### Error Boundaries

Show an error component when navigation fails:

```html
<route path="dashboard" component="dashboard-page" exact error="error-display" />
```

The error component receives `{ error, status, path }` as state. It can call `Router.navigate()` to retry.

### Controlling State

| Need | Mechanism |
|------|-----------|
| **Server provides all state** (default) | No changes needed |
| **I fetch my own data** | `static loader()` on component class |
| **Preserve local state** | `keep-alive` on route |
| **Preserve DOM, refresh data** | `keep-alive` + `static loader()` |
| **Handle form submissions** | `static action()` on component class |
| **Cache responses** | `cache` config on `Router.start()` |
| **Show loading state** | `pending` attr on route |
| **Handle failures** | `error` attr on route |

## API

### `Router.start(config?)`

Start the router. Call after hydration completes.

| Option | Type | Description |
|--------|------|-------------|
| `loaders` | `Record<string, () => Promise<unknown>>` | Lazy-loading map: component tag -> dynamic import |
| `templateEndpoint` | `string` | URL for `ensureLoaded()` requests (default: `"/_webui/templates"`) |
| `dev` | `boolean` | Enable development mode warnings |
| `preload` | `boolean` | Preload routes on link hover for instant navigation |
| `ssrFresh` | `boolean` | Skip initial loader replay on SSR bootstrap (default: `true`). Components with `static ssrLoader = true` still run their loader. |
| `cache` | `CacheConfig` | Tagged navigation cache: `{ staleTime, gcTime, maxEntries }` |

### `Router.navigate(path)`

Programmatic navigation:

```typescript
Router.navigate('/users/42');
```

### `Router.ensureLoaded(...tags)`

Load templates and CSS for components on demand. Components must be declared
as routes so the build compiles them, but they don't need to be navigated to:

```html
<!-- Declared as a route — compiled into protocol -->
<route path="settings" component="settings-dialog" exact />
```

```typescript
// Load on demand — fetches from /_webui/templates
await Router.ensureLoaded('settings-dialog');

// Batch multiple in one request
await Router.ensureLoaded('modal-a', 'modal-b', 'drawer-c');
```

Templates are not sent during initial SSR or partial navigation for
unmatched routes — zero cost until explicitly requested. If a user navigates
directly to the route path, the component renders normally in the outlet.

### View Transitions

The router automatically uses the [View Transitions API](https://developer.mozilla.org/en-US/docs/Web/API/Document/startViewTransition) when available. On each client-side navigation, the DOM swap is wrapped in `document.startViewTransition()`, giving you a CSS-driven cross-fade between old and new route content with zero extra code.

**Do NOT wrap `Router.navigate()` in your own `startViewTransition()`** — the router already does this internally.

To customize the animation, use `view-transition-name` on specific elements and target them in CSS:

```css
/* Scope the transition to the reading pane only */
.route-outlet {
  view-transition-name: reading-pane;
}

/* Custom cross-fade for the reading pane */
::view-transition-old(reading-pane) {
  animation: fade-out 100ms ease-out;
}
::view-transition-new(reading-pane) {
  animation: fade-in 150ms ease-in;
}
```

The router awaits `transition.updateCallbackDone` (not `.finished`), so rapid navigations supersede each other without queuing animations.

### `Router.back()`

Navigate back in history.

### `Router.invalidateTags(tags)`

Evict all cache entries whose tags overlap with the given tags:

```typescript
Router.invalidateTags(['inbox', 'thread:42']);
```

### `Router.invalidate(path?)`

Evict cache entries by path, or all entries if no path is given:

```typescript
Router.invalidate('/email/42');  // one entry
Router.invalidate();             // everything
```

### `Router.activeComponent`

Component tag of the active leaf route:

```typescript
console.log(Router.activeComponent); // "user-detail"
```

### `Router.activeParams`

Bound parameters from all nesting levels:

```typescript
console.log(Router.activeParams); // { id: "42" }
```

### `Router.destroy()`

Tear down the router and remove event listeners.

### `Router.gc(tags?)`

Release cached component templates to free memory. Removes entries from
`window.__webui.templates` and `window.__webui.templateFns`, then clears
their inventory bits so the server will re-send them on the next navigation
that needs them.

```typescript
// Release all cached templates
Router.gc();
```

The framework's internal `templateCache` (`WeakMap`) is keyed by the same
meta objects, so its entries become GC-eligible automatically.

### Navigation Events

Dispatched on `window` after each navigation:

```typescript
window.addEventListener('webui:route:navigated', (e) => {
  const { component, params, query, path } = (e as CustomEvent).detail;
});
```

Dispatched after a mutation action completes:

```typescript
window.addEventListener('webui:route:action-complete', (e) => {
  const { component, invalidatedTags, path } = (e as CustomEvent).detail;
});
```

> **Note:** The `query` in `navigated` contains **all** URL query parameters (unfiltered). Only
> parameters declared via the `query` attribute on `<route>` are set as DOM
> attributes — the event exposes the full set for programmatic use.

## Route Path Syntax

| Pattern | Example | Matches |
|---------|---------|---------|
| `literal` | `users` | Exact segment |
| `:param` | `users/:id` | Captures segment → `{ id: "42" }` |
| `:param?` | `search/:query?` | Optional segment |
| `*splat` | `files/*path` | Rest of path → `{ path: "a/b/c" }` |

Paths are relative to the parent route. Use `/` prefix only for the root route.

## Query Parameters

URL query parameters can be forwarded to components as HTML attributes by
declaring an **allowlist** on the `<route>` element:

```html
<route path="compose" component="page-compose" query="action,to,subject" exact />
```

When a user navigates to `/compose?action=reply&to=user@test.com`, only the
three listed parameters are set as attributes on `<page-compose>`. Any
unlisted parameter (e.g. `?class=evil&style=display:none`) is silently
dropped — **deny-by-default**.

### Rules

| Scenario | Behavior |
|----------|----------|
| No `query` attribute | No query params forwarded (safe default) |
| `query="action,to"` | Only `action` and `to` are set as attributes |
| Collision with route param | Route param wins — query param is skipped |
| Query-only navigation | Stale attributes from previous query are removed |

### Component usage

Declare `@attr` properties matching the allowed query param names:

```typescript
export class PageCompose extends WebUIElement {
  @attr action = '';
  @attr to = '';
  @attr subject = '';
}
```

## Server Contract

On client-side navigation, the router sends:

```
GET /users/42
Accept: application/x-ndjson, application/json
X-WebUI-Inventory: <hex bitmask>
```

The server should return:

- **`Accept: application/x-ndjson`** → NDJSON streaming: Chunk 1 `{ templateStyles, templates, inventory, path, chain, cacheTags }`, Chunk 2 `{ states: [...] }` — or fall back to single JSON
- **`Accept: application/json`** → JSON partial: `{ state, templateStyles, templates, inventory, path, chain, cacheTags, cacheControl }` — `state` is added by the caller; `render_partial()` returns everything else
- **Otherwise** → Full SSR'd HTML page

The `chain` field contains the matched route chain with `component`, `path`, `params`, `exact`, `keepAlive`, `pendingComponent`, `errorComponent`, and `invalidates`. The `cacheTags` array contains resolved cache tags from the full chain. The optional `cacheControl` object can override `staleTime` per-response.

See the [Routing guide](https://github.com/microsoft/webui/blob/main/docs/guide/concepts/routing.md) for complete server implementation examples.

## Architecture

The router is organized into 13 internal modules, each handling a single concern:

| Module | Responsibility |
|--------|---------------|
| `router` | Core router lifecycle, Navigation API integration |
| `chain` | SSR chain parsing, `window.__webui` bootstrap, `data-ri` binding |
| `navigation-path` | Path matching and parameter extraction |
| `route-element` | `<webui-route>` custom element and query param handling |
| `loaders` | Static `loader()` resolution with `ssrFresh` support |
| `actions` | Form submission interception and `static action()` dispatch |
| `cache` | Tagged LRU navigation cache |
| `pending` | Pending UI threshold and lifecycle |
| `preload` | Hover-based speculative prefetching |
| `templates` | Template injection and inventory management |
| `streaming` | NDJSON streaming partial responses |
| `browser-shim` | Navigation API type shims |
| `types` | Public type definitions and type guards |

## SSR Bootstrap (`window.__webui`)

On first load, the server emits inert SSR metadata in `#webui-data`; a tiny bootstrap script parses it into `window.__webui` and installs condition closures:

```html
<script type="application/json" id="webui-data">
{
  "chain": [],
  "inventory": "04000400...",
  "nonce": "abc123",
  "css": ["/styles/main.css"],
  "styles": ["app-shell"],
  "templates": {}
}
</script>
```

The router reads this at startup, eliminating DOM walking and URLPattern usage. Older servers that emit `<meta name="webui-inventory">` are still supported as a fallback.

## Exports

The package exports the following:

| Export | Kind | Description |
|--------|------|-------------|
| `Router` | class | Main router singleton |
| `WebUIRouter` | class | Same as `Router` (named export) |
| `WebUIRouteElement` | class | `<webui-route>` custom element |
| `parseQuery` | function | Parse URL query string into a record |
| `filterQuery` | function | Filter query params by an allowlist |
| `isStateful` | function | Type guard - checks if an element implements `setState()` |
| `StatefulElement` | type | Interface for elements with `setState()` support |
| `RouterConfig` | type | Configuration for `Router.start()` |
| `RouteLoaderContext` | type | Context passed to `static loader()` methods |
| `RouteActionContext` | type | Context passed to `static action()` methods |
| `RouteActionResult` | type | Return type of `static action()` |
| `CacheConfig` | type | Cache configuration options |
| `NavigationEvent` | type | Detail type for `webui:route:navigated` events |
| `ActionCompleteEvent` | type | Detail type for `webui:route:action-complete` events |

## License

MIT
