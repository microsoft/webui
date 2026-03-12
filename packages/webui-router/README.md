# @microsoft/webui-router

Lightweight client-side router for [WebUI](https://github.com/microsoft/webui) apps. Uses the [Navigation API](https://developer.mozilla.org/en-US/docs/Web/API/Navigation_API) to intercept navigations, matches routes defined with `<route>` directives, and loads components on demand — preserving server-rendered content on initial page load and fetching JSON partials for subsequent navigations.

## How It Works

1. **Server renders the initial page** — the matched route's component is fully SSR'd with a declarative shadow root. The page is interactive before any JavaScript loads.
2. **Hydration completes** — FAST-HTML hydrates the shell components (nav, sidebar, etc.).
3. **Router starts** — intercepts link clicks via the Navigation API.
4. **Client-side navigation** — fetches a JSON partial (`{ state, templates }`) from the server, lazy-loads the route component's JS if needed, and mounts it into the `<webui-route>` element.

No full page reloads. The shell stays in place. Only the route content changes.

## Installation

```bash
npm install @microsoft/webui-router
```

## Quick Start

Define routes in your HTML:

```html
<route path="/" name="home" component="home-page" exact />
<route path="/users" name="users" component="user-list" exact />
<route path="/users/:id" name="detail" component="user-detail" exact />
```

Start the router after hydration:

```typescript
import { TemplateElement } from '@microsoft/fast-html';
import { Router } from '@microsoft/webui-router';

import './home-page.js'; // eagerly load the shell page

TemplateElement.options({
  'home-page': { observerMap: 'all' },
}).config({
  hydrationComplete() {
    Router.start({
      loaders: {
        'user-list': () => import('./user-list.js'),
        'user-detail': () => import('./user-detail.js'),
      },
    });
  },
}).define({ name: 'f-template' });
```

Enable code splitting in your bundler so dynamic `import()` calls produce separate chunks:

```bash
esbuild src/index.ts --bundle --splitting --outdir=dist --format=esm
```

## API

### `Router.start(config?)`

Start the router. Options:

| Option | Type | Description |
|--------|------|-------------|
| `basePath` | `string` | Prefix for all route URLs (e.g., `"/app"`) |
| `loaders` | `Record<string, () => Promise<unknown>>` | Lazy-loading map: component tag → dynamic import |

### `Router.navigate(path)`

Programmatic navigation.

### `Router.back()`

Navigate back in history.

### `Router.activeRouteName`

Name of the currently active route.

### `Router.activeParams`

Bound parameters of the current route (e.g., `{ id: "42" }`).

### `Router.destroy()`

Tear down the router and remove event listeners.

### Navigation Event

The router dispatches `webui:route:navigated` on `window` after each navigation:

```typescript
window.addEventListener('webui:route:navigated', (e) => {
  const { routeName, params, path } = e.detail;
});
```

## Route Path Syntax

| Pattern | Example | Matches |
|---------|---------|---------|
| `/literal` | `/users` | Exact segment |
| `/:param` | `/users/:id` | Captures segment → `{ id: "42" }` |
| `/:param?` | `/search/:query?` | Optional segment |
| `/*splat` | `/files/*path` | Rest of path → `{ path: "a/b/c" }` |

Add the `exact` attribute to require full path match (no prefix matching).

## Server Contract

On client-side navigation, the router sends:

```
GET /users/42
Accept: application/json
X-WebUI-Inventory: <hex bitmask>
```

The server should return:

- **`Accept: application/json`** → JSON partial: `{ state, templates, inventory, path }`
- **Otherwise** → Full SSR'd HTML page

See the [Routing guide](https://github.com/microsoft/webui/blob/main/docs/guide/concepts/routing.md) for complete server implementation examples.

## License

MIT
