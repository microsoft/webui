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
<template shadowrootmode="open">
  <nav>
    <a href="/">Home</a>
    <a href="/users">Users</a>
  </nav>
  <main><outlet /></main>
  <footer>© 2026</footer>
</template>
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
  const { component, params, path } = (e as CustomEvent).detail;
  console.log(`Navigated to ${component}`, params);
});
```

## Route Path Syntax

| Pattern | Example | Matches |
|---------|---------|---------|
| `literal` | `users` | Exact segment |
| `:param` | `users/:id` | Captures segment → `{ id: "42" }` |
| `:param?` | `search/:query?` | Optional segment |
| `*splat` | `files/*path` | Rest of path → `{ path: "a/b/c" }` |

Paths are relative to the parent route. Use `/` prefix only for the root route.

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
