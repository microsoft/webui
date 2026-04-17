# Routing

WebUI provides nested SSR routing with client-side navigation. Routes are declared as a tree in `index.html`, components use `<outlet />` to render child routes, and the `@microsoft/webui-router` package handles client-side transitions after the initial SSR.

## Installation

```bash
npm install @microsoft/webui-router
```

Only needed when your app has client-side navigation. Server-only apps with full page loads don't need it.

## Quick Start

**1. Declare routes in `index.html`:**

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

## How It Works

### First Load (SSR)

1. Browser requests `/sections/1/topics/react`
2. Server matches the full route chain: `app-shell → section-page → topic-page`
3. Renders all matched components nested at their outlets
4. Browser displays fully rendered HTML - no JavaScript needed yet
5. JavaScript loads, hydration runs, router starts and reads the SSR'd active chain

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
  basePath: '/app',   // optional: prefix for all route URLs
  loaders: { ... },   // optional: lazy-loading map
});
```

### `Router.navigate(path)`

```typescript
Router.navigate('/users/42');
```

### `Router.back()`

```typescript
Router.back();
```

### `Router.activeParams`

The bound parameters of the current route:

```typescript
console.log(Router.activeParams); // { id: "42" }
```

### `Router.destroy()`

Tears down the router and removes event listeners.

### `Router.gc()`

Release all cached component templates to free memory. Removes all entries from `window.__webui_templates` and clears their inventory bits so the server will re-send them on the next navigation that needs them.

```typescript
// Release all non-active templates
Router.gc();
```

The framework's internal template cache is a `WeakMap` keyed by the same meta objects, so its entries become GC-eligible automatically when the template is released.

::: tip When to use this
Most apps don't need this - the number of unique component templates is bounded by the route tree (typically 10–30). The server's inventory system already prevents duplicate downloads. Use `gc()` in long-lived SPAs with many routes where memory pressure is a concern.
:::

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
  const { component, params, path } = event.detail;
});
```

## Server Contract

The server handles two request types for each route:

### JSON Partial (client navigation)

When `Accept: application/json`:

```json
{
  "state": { "name": "Alice", "email": "alice@example.com" },
  "templateStyles": ["<style type=\"module\" specifier=\"user-detail\">.user-detail{display:block}</style>"],
  "templates": ["(function(){var w=window.__webui_templates||...})();"],
  "inventory": "04000400...",
  "path": "/users/42",
  "chain": [
    { "component": "app-shell", "path": "/" },
    { "component": "user-detail", "path": "users/:id", "params": { "id": "42" }, "exact": true }
  ]
}
```

The `templateStyles` array contains module CSS definition tags for CssStrategy::Module. The client appends these to `<head>` before evaluating template scripts so adopted stylesheets are available. For Link/Style modes, this array is empty.

The `chain` field tells the client router which route components are active at each nesting level. The client uses this to diff against the previous chain and only remount what changed - it does **not** perform route matching itself.

### Full HTML (initial load)

Without `Accept: application/json`, return the full SSR'd page. The handler automatically emits a `<meta name="webui-inventory">` tag in `<head>` so the client router knows which templates are already loaded.

### Building the chain

Use `render_partial()` (Rust) or `webui_render_partial()` (FFI) to get the complete partial response - state, templateStyles, templates, inventory, path, and chain - in a single call:

```rust
// Rust
let partial = route_handler::render_partial(&protocol, state, &entry, &path, &inventory_hex);
// partial contains: { "state": {...}, "templateStyles": [...], "templates": [...], "inventory": "...", "path": "...", "chain": [...] }
```

```csharp
// C#
string partialJson = handler.RenderPartial(protocol, stateJson, entryId, requestPath, inventoryHex);
```

```javascript
// Node.js
const partialJson = webui.renderPartial(protocol, stateJson, entryId, requestPath, inventoryHex);
```

### Express Example

```javascript
app.get('/users/:id', (req, res) => {
  const state = { name: getUser(req.params.id).name };

  if (req.accepts('json')) {
    // renderPartial() returns the complete response - no assembly required
    const stateJson = JSON.stringify(state);
    res.type('json').send(webui.renderPartial(protocol, stateJson, 'index.html', req.path, req.get('X-WebUI-Inventory') ?? ''));
  } else {
    res.type('html').send(handler.render(protocol, state, 'index.html', req.path));
  }
});
```

## Security

Route parameters (`:id`, `:name`, etc.) are extracted from URLs and injected into component state. They are automatically HTML-escaped when rendered with double braces (<code v-pre>{{param}}</code>), but **not** when rendered with triple braces (<code v-pre>{{{param}}}</code>).

> ⚠️ Never use triple braces (<code v-pre>{{{...}}}</code>) to render route parameters. An attacker could craft a URL like `/users/<script>alert(1)</script>` to inject arbitrary HTML.

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
        'contacts-page': () => import('./pages/contacts-page.js'),
        'contact-form': () => import('./pages/contact-form.js'),
        'contact-detail': () => import('./pages/contact-detail.js'),
      },
    });
  },
}).define({ name: 'f-template' });
```
