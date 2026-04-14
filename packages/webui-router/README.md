# @microsoft/webui-router

Client-side router for [WebUI](https://github.com/microsoft/webui) apps with nested route support. Uses the [Navigation API](https://developer.mozilla.org/en-US/docs/Web/API/Navigation_API) to intercept link clicks and loads components on demand — preserving server-rendered content on initial load and fetching JSON partials for subsequent navigations. The server provides the matched route chain; the client does not perform route matching.

> 📖 **Full documentation at [microsoft.github.io/webui](https://microsoft.github.io/webui)** — see the [Routing Guide](https://microsoft.github.io/webui/guide/concepts/routing) for setup and usage.

## How It Works

1. **Server renders the full page** — the matched route chain is SSR'd with declarative shadow roots. The page is interactive before JavaScript loads.
2. **Hydration completes** — FAST-HTML hydrates shell components.
3. **Router starts** — reads the SSR'd active chain and intercepts link clicks via the Navigation API.
4. **Client-side navigation** — fetches a JSON partial from the server, which includes the matched route chain. The client diffs old vs new chain and mounts only the changed component. Parent components stay mounted.

No full page reloads. The shell stays in place. Only route content changes.

## Installation

```bash
npm install @microsoft/webui-router
```

## Quick Start

**1. Declare nested routes in `index.html`:**

```html
<body>
  <route path="/" component="app-shell">
    <route path="" component="home-page" exact />
    <route path="users" component="user-list" exact />
    <route path="users/:id" component="user-detail" exact />
  </route>
  <script type="module" src="/index.js"></script>
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
import { TemplateElement } from '@microsoft/fast-html';
import { Router } from '@microsoft/webui-router';

import './app-shell.js';

TemplateElement.options({
  'app-shell': { observerMap: 'all' },
}).config({
  hydrationComplete() {
    Router.start({
      loaders: {
        'home-page': () => import('./pages/home-page.js'),
        'user-list': () => import('./pages/user-list.js'),
        'user-detail': () => import('./pages/user-detail.js'),
      },
    });
  },
}).define({ name: 'f-template' });
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

## API

### `Router.start(config?)`

Start the router. Call after hydration completes.

| Option | Type | Description |
|--------|------|-------------|
| `basePath` | `string` | Prefix for all route URLs (e.g., `"/app"`) |
| `loaders` | `Record<string, () => Promise<unknown>>` | Lazy-loading map: component tag → dynamic import |

### `Router.navigate(path)`

Programmatic navigation:

```typescript
Router.navigate('/users/42');
```

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

Release cached component templates to free memory. Removes all entries from
`window.__webui_templates` and clears their inventory bits so the server
will re-send them on the next navigation that needs them.

Active route components are always skipped — you cannot release a template
that is currently rendered.

```typescript
// Release all non-active templates
Router.gc();
```

The framework's internal `templateCache` (`WeakMap`) is keyed by the same
meta objects, so its entries become GC-eligible automatically.

### Navigation Event

Dispatched on `window` after each navigation:

```typescript
window.addEventListener('webui:route:navigated', (e) => {
  const { component, params, query, path } = (e as CustomEvent).detail;
  console.log(`Navigated to ${component}`, params, query);
});
```

> **Note:** `query` contains **all** URL query parameters (unfiltered). Only
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
Accept: application/json
X-WebUI-Inventory: <hex bitmask>
```

The server should return:

- **`Accept: application/json`** → JSON partial: `{ state, templateStyles, templates, inventory, path, chain }` - returned directly from `renderPartial()`, no assembly required
- **Otherwise** → Full SSR'd HTML page (handler emits a `<meta name="webui-inventory">` tag in `<head>` so the client knows which templates are loaded)

When `templateStyles` is present (CssStrategy::Module), the router appends those module CSS definition tags to `<head>` before evaluating the batched template scripts. The `chain` field contains the matched route chain - the client uses it to diff against the previous chain and only remount what changed. The `X-WebUI-Inventory` header is a bitmask of component templates the client already has - the server uses it to avoid sending duplicate templates.

See the [Routing guide](https://github.com/microsoft/webui/blob/main/docs/guide/concepts/routing.md) for complete server implementation examples.

## License

MIT
