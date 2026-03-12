# `<route>` Directive

The `<route>` directive defines a client-side route that maps a URL path to a component. At build time, `<route>` elements are compiled into `<webui-route>` custom elements. At runtime, the server renders the matched route's component and the client router handles subsequent navigations.

## Basic Usage

```html
<main>
  <route path="/" name="home" component="home-page" exact />
  <route path="/users" name="users" component="user-list" exact />
  <route path="/users/:id" name="user-detail" component="user-detail" exact />
</main>
```

The server matches the request path against the defined routes and renders the matching component with its state. Non-matching routes are rendered hidden.

## Attributes

| Attribute | Required | Description |
|-----------|----------|-------------|
| `path` | Yes | URL path template to match (e.g., `/users/:id`) |
| `component` | Yes | Tag name of the component to render (e.g., `user-detail`) |
| `name` | No | Unique name for the route (used in navigation events) |
| `exact` | No | Only match when the path matches exactly (no prefix matching) |

## Path Parameters

Route paths support dynamic segments that capture values from the URL:

### Required Parameters

Use `:name` to capture a path segment:

```html
<route path="/users/:id" component="user-detail" exact />
```

Matches `/users/42` → `{ id: "42" }`

### Optional Parameters

Use `:name?` for optional segments:

```html
<route path="/search/:query?" component="search-page" exact />
```

Matches both `/search` and `/search/hello` → `{ query: "hello" }`

### Splat (Catch-all)

Use `*name` to capture the rest of the path:

```html
<route path="/files/*path" component="file-browser" />
```

Matches `/files/docs/readme.md` → `{ path: "docs/readme.md" }`

## Route Specificity

When multiple routes can match a path, WebUI picks the most specific one — the route with the most literal (non-parameter) segments wins.

```html
<route path="/users/add" component="user-form" exact />
<route path="/users/:id" component="user-detail" exact />
```

A request to `/users/add` matches the first route (2 literal segments) over the second (1 literal + 1 param).

## Exact vs Prefix Matching

By default, routes use prefix matching — `/users` matches `/users`, `/users/42`, and `/users/42/edit`. Add the `exact` attribute to require a full match:

```html
<!-- Prefix: matches /app, /app/settings, /app/anything -->
<route path="/app" component="app-shell" />

<!-- Exact: only matches /app/settings -->
<route path="/app/settings" component="settings-page" exact />
```

## Inside Components

Routes are typically placed inside a shell component's template:

```html
<!-- app-shell.html -->
<template>
  <header><nav-bar></nav-bar></header>
  <main>
    <route path="/" name="dashboard" component="dashboard-page" exact />
    <route path="/contacts" name="contacts" component="contacts-page" exact />
    <route path="/contacts/:id" name="detail" component="contact-detail" exact />
  </main>
</template>
```

The shell component is rendered on every page. Only the matched route's component changes.

## Server-Side Rendering

When the server receives a request, it matches the URL against the route definitions and:

1. **Matched route** — rendered visible with its component's full HTML content (including declarative shadow root)
2. **Non-matched routes** — rendered hidden (`style="display:none"`) with no content

This means the browser displays the correct page instantly, before any JavaScript loads.

## Notes and Limitations

- Route elements are converted to `<webui-route>` custom elements at build time
- Routes can be placed in the light DOM or inside a component's shadow DOM
- The server performs route matching during SSR — no client JavaScript is needed for the initial render
- For client-side navigation between routes, install the [`@microsoft/webui-router`](/guide/concepts/routing) package
- Self-closing syntax (`<route ... />`) and open/close syntax (`<route ...>...</route>`) are both supported
