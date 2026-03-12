# Routing

WebUI includes a lightweight client-side router that works with the `<route>` directive to enable single-page navigation. The server renders the initial page with full SSR, then the router takes over for subsequent navigations — fetching only the data and templates needed for the new route.

## Installation

Install the router package alongside your WebUI dependencies:

```bash
npm install @microsoft/webui-router
```

The router is a separate package because it's only needed when your app has client-side navigation. Server-only apps that do full page loads on every request don't need it.

## Quick Start

Define routes in your HTML template:

```html
<nav>
  <a href="/">Dashboard</a>
  <a href="/users">Users</a>
</nav>
<main>
  <route path="/" name="dashboard" component="dashboard-page" exact />
  <route path="/users" name="users" component="user-list" exact />
  <route path="/users/:id" name="detail" component="user-detail" exact />
</main>
<script type="module" src="/index.js"></script>
```

Start the router in your entry point:

```typescript
import { Router } from '@microsoft/webui-router';

Router.start();
```

That's it. The server SSRs the matched route on first load, and the router handles clicks on `<a>` tags for all subsequent navigations.

## How It Works

### First Page Load (SSR)

1. Browser requests `/users`
2. Server matches the URL against routes, renders `user-list` as the active route
3. Browser displays the fully rendered HTML — no JavaScript needed yet
4. JavaScript loads, hydration runs, router starts
5. Router sees the SSR'd content is already correct and preserves it

### Client-Side Navigation

1. User clicks a link to `/users/42`
2. Router intercepts via the [Navigation API](https://developer.mozilla.org/en-US/docs/Web/API/Navigation_API)
3. Router sends `fetch("/users/42")` with `Accept: application/json`
4. Server returns a JSON partial with state + component templates
5. Router loads the component JS (if lazy), mounts it, and passes the state

No full page reload. The shell (nav, header, sidebar) stays in place.

## The `Router` Class

### `Router.start(config?)`

Starts the router. Call this after hydration completes.

```typescript
import { Router } from '@microsoft/webui-router';

Router.start({
  basePath: '/app',   // optional: prefix for all route URLs
  loaders: { ... },   // optional: lazy-loading map
});
```

### `Router.navigate(path)`

Programmatically navigate to a route:

```typescript
Router.navigate('/users/42');
```

### `Router.back()`

Navigate back in history:

```typescript
Router.back();
```

### `Router.activeRouteName`

The name of the currently active route:

```typescript
console.log(Router.activeRouteName); // "detail"
```

### `Router.activeParams`

The bound parameters of the current route:

```typescript
console.log(Router.activeParams); // { id: "42" }
```

### `Router.destroy()`

Tears down the router, removes event listeners:

```typescript
Router.destroy();
```

## Configuration

The `RouterConfig` interface:

```typescript
interface RouterConfig {
  /** Base path prepended to all route URLs. */
  basePath?: string;

  /** Lazy-loading map: component tag → async loader function. */
  loaders?: Record<string, () => Promise<unknown>>;
}
```

## Lazy Loading with Dynamic Imports

By default, all route component JavaScript is bundled into your entry file and loaded on first page visit. For apps with many pages, you can lazy-load route components so their JavaScript is only fetched when the user navigates to that route.

### Setup

1. **Remove static imports** for page components from your entry file
2. **Add loaders** to `Router.start()` mapping component tags to dynamic imports
3. **Enable code splitting** in your bundler

```typescript
import { TemplateElement } from '@microsoft/fast-html';
import { Router } from '@microsoft/webui-router';

// Only import shell components eagerly
import './app-shell.js';
import './nav-bar.js';

TemplateElement.options({
  'app-shell': { observerMap: 'all' },
  'nav-bar': { observerMap: 'all' },
}).config({
  hydrationComplete() {
    Router.start({
      loaders: {
        'dashboard-page': () => import('./pages/dashboard-page.js'),
        'user-list': () => import('./pages/user-list.js'),
        'user-detail': () => import('./pages/user-detail.js'),
      },
    });
  },
}).define({ name: 'f-template' });
```

### Bundler Configuration

Enable code splitting so dynamic `import()` calls produce separate chunks:

```bash
# esbuild
esbuild src/index.ts --bundle --splitting --outdir=dist --format=esm

# vite (splitting is on by default)
vite build
```

### How It Works

- Components **not in `loaders`** are assumed to be eagerly loaded (imported statically)
- Components **in `loaders`** are loaded on demand when the user navigates to that route
- Each loader runs **at most once** — the promise is cached
- On SSR'd initial page load, the matched route's content is already rendered — the lazy loader is **skipped** to preserve the server-rendered DOM
- Lazy loading is **opt-in** — omitting `loaders` entirely preserves the default eager behavior

## Navigation Events

The router dispatches a `webui:route:navigated` event on `window` after every navigation:

```typescript
window.addEventListener('webui:route:navigated', (event) => {
  const { routeName, params, path } = event.detail;
  console.log(`Navigated to ${routeName} at ${path}`, params);
});
```

The `NavigationEvent` detail:

```typescript
interface NavigationEvent {
  routeName: string;                  // e.g., "detail"
  params: Record<string, string>;     // e.g., { id: "42" }
  path: string;                       // e.g., "/users/42"
}
```

## Server Contract for Route Navigation

When the client router navigates, it fetches the route URL with special headers. The server must handle two cases:

### JSON Partial (client-side navigation)

When the request includes `Accept: application/json`, the server should return a JSON object with the route's state and any component templates the client doesn't have yet:

**Request:**
```
GET /users/42
Accept: application/json
X-WebUI-Inventory: 04000000000000000000...
```

**Response:**
```json
{
  "state": {
    "name": "Alice Johnson",
    "email": "alice@example.com",
    "role": "Admin"
  },
  "templates": [
    "<f-template name=\"user-detail\">...</f-template>"
  ],
  "inventory": "04000400000000000000...",
  "path": "/users/42"
}
```

| Field | Description |
|-------|-------------|
| `state` | JSON object passed to the component's `setInitialState(state, params)` method |
| `templates` | Array of `<f-template>` HTML strings for components the client doesn't have yet |
| `inventory` | Updated hex bitmask tracking which component templates are loaded |
| `path` | The matched route path |

The `X-WebUI-Inventory` header is a bloom-filter bitmask (256-bit, hex-encoded) that tells the server which component templates the client already has. The server uses this to skip sending templates the client doesn't need. If the header is missing, send all templates.

### Full HTML (initial page load / direct navigation)

When the request does **not** include `Accept: application/json` (normal browser navigation), the server should return the full SSR'd HTML page — the same as any initial page load.

### Implementation Example (Node/Express)

```javascript
app.get('/users/:id', (req, res) => {
  const user = getUser(req.params.id);
  const state = { name: user.name, email: user.email };

  if (req.accepts('json')) {
    // Client-side navigation — return JSON partial
    const inventory = req.headers['x-webui-inventory'] || '';
    const { templates, updatedInventory } = getRouteTemplates(
      protocol, 'user-detail', inventory
    );
    res.json({
      state,
      templates,
      inventory: updatedInventory,
      path: req.path,
    });
  } else {
    // Full page load — render complete HTML
    const html = handler.render(protocol, state, {
      entry: 'index.html',
      requestPath: req.path,
    });
    res.type('html').send(html);
  }
});
```

### Implementation Example (C# / .NET)

```csharp
app.MapGet("/users/{id}", (string id, HttpContext ctx) =>
{
    var user = GetUser(id);
    var state = new { name = user.Name, email = user.Email };

    if (ctx.Request.Headers.Accept.Contains("application/json"))
    {
        var inventory = ctx.Request.Headers["X-WebUI-Inventory"].FirstOrDefault() ?? "";
        var (templates, updatedInventory) = webui.GetRouteTemplates(
            protocol, "user-detail", inventory
        );
        return Results.Json(new {
            state, templates, inventory = updatedInventory, path = ctx.Request.Path
        });
    }

    var html = webui.Render(protocol, state, "index.html", ctx.Request.Path);
    return Results.Content(html, "text/html");
});
```

## Full Example

A complete multi-page app with lazy-loaded routes:

**`src/index.html`**
```html
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <title>My App</title>
</head>
<body>
  <nav>
    <a href="/">Home</a>
    <a href="/about">About</a>
    <a href="/users">Users</a>
  </nav>
  <main>
    <route path="/" name="home" component="home-page" exact />
    <route path="/about" name="about" component="about-page" exact />
    <route path="/users" name="users" component="user-list" exact />
    <route path="/users/:id" name="detail" component="user-detail" exact />
  </main>
  <script type="module" src="/index.js"></script>
</body>
</html>
```

**`src/index.ts`**
```typescript
import { TemplateElement } from '@microsoft/fast-html';
import { Router } from '@microsoft/webui-router';

// Eagerly load the home page (SSR'd on first visit)
import './home-page.js';

TemplateElement.options({
  'home-page': { observerMap: 'all' },
}).config({
  hydrationComplete() {
    Router.start({
      loaders: {
        'about-page': () => import('./about-page.js'),
        'user-list': () => import('./user-list.js'),
        'user-detail': () => import('./user-detail.js'),
      },
    });
  },
}).define({ name: 'f-template' });
```

**Build:**
```bash
webui build ./src --out ./dist --plugin=fast
esbuild src/index.ts --bundle --splitting --outdir=dist --format=esm
```
