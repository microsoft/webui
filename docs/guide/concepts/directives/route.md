# `<route>` Directive

The `<route>` directive defines a URL route that maps a path to a component. Routes are declared in the entry HTML (`index.html`) as a nested tree. At build time, they're compiled into `<webui-route>` custom elements. The server renders the matched route chain via SSR, and the client router handles subsequent navigations.

## Declaring Routes

Routes are declared in `index.html` as nested elements. The HTML nesting defines the route hierarchy:

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

Child route paths are **relative to their parent** - no leading `/` needed.

## The `<outlet />` Directive

Components that have child routes use `<outlet />` to mark where the matched child renders:

```html
<!-- app-shell.html -->
<header><nav-bar></nav-bar></header>
<main>
  <outlet />
</main>
<footer></footer>
```

The shell (header, footer) persists across all routes. Only the content at `<outlet />` changes.

## Nested Routes

Routes can be nested to any depth. Each level's component uses `<outlet />` for its children:

```html
<!-- index.html -->
<route path="/" component="app-shell">
  <route path="" component="dashboard" exact />
  <route path="contacts" component="contacts-page">
    <route path="add" component="contact-form" exact />
    <route path=":id" component="contact-detail" exact />
    <route path=":id/edit" component="contact-form" exact />
  </route>
</route>
```

```html
<!-- contacts-page.html -->
<h2>Contacts</h2>
<div class="contact-list">...</div>
<outlet />
```

Navigating from `/contacts/1` to `/contacts/2` preserves the contacts list - only the detail view at `<outlet />` changes.

## Attributes

| Attribute | Required | Description |
|-----------|----------|-------------|
| `path` | Yes | URL path segment to match (relative to parent) |
| `component` | Yes | Tag name of the component to render |
| `exact` | No | Only match when the full path is consumed (no prefix matching) |
| `query` | No | Comma-separated allowlist of query params forwarded as component attributes (deny-by-default) |

## Path Parameters

### Required: `:name`
```html
<route path="users/:id" component="user-detail" exact />
```
`/users/42` → `{ id: "42" }`

### Optional: `:name?`
```html
<route path="search/:query?" component="search-page" exact />
```
Matches `/search` and `/search/hello`

### Catch-all: `*name`
```html
<route path="files/*path" component="file-browser" />
```
`/files/docs/readme.md` → `{ path: "docs/readme.md" }`

## Route Specificity

When multiple sibling routes match, the most specific one wins (most literal segments):

```html
<route path="users/add" component="user-form" exact />
<route path="users/:id" component="user-detail" exact />
```

`/users/add` matches the first route (2 literals) over the second (1 literal + 1 param).

## Security

Route parameters (`:id`, `:name`, etc.) are extracted from URLs and injected into component state. They are automatically HTML-escaped when rendered with double braces (`{{param}}`), but **not** when rendered with triple braces (`{{{param}}}`).

> ⚠️ Never use triple braces (`{{{...}}}`) to render route parameters. An attacker could craft a URL like `/users/<script>alert(1)</script>` to inject arbitrary HTML.

Always validate route parameters on the server before including them in state.

## Exact vs Prefix Matching

Without `exact`, a route matches any URL that starts with its path. Parent routes with children should omit `exact`. Leaf routes should use `exact`:

```html
<route path="/" component="app-shell">          <!-- prefix: matches everything -->
  <route path="settings" component="settings">   <!-- prefix: /settings/* -->
    <route path="profile" component="profile" exact />  <!-- exact: only /settings/profile -->
  </route>
</route>
```

## SSR Behavior

On the server:
1. **Matched routes** - rendered visible with full component content
2. **Non-matched siblings** - rendered hidden (`style="display:none"`) with no content
3. The browser displays the correct page instantly, before JavaScript loads
