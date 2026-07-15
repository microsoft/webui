---
layout: page
---

# WebUI Framework - AI Reference

> **Single-page reference for LLMs.** This page contains everything an AI coding
> assistant needs to generate correct WebUI Framework code. It covers the SSR
> model, template syntax, component authoring, CLI commands, and known
> constraints. Bookmark this page and feed it to your AI when working with
> WebUI.

## What is WebUI Framework?

WebUI is a **language-agnostic server-side rendering framework**. Templates
are compiled to a binary Protocol Buffer at build time. At runtime, any
backend (Rust, Node, Go, C#, Python) fills in JSON state and produces HTML.
On the client, interactive Web Components hydrate as islands.

**Key facts an AI must know:**

- The server renders HTML from a compiled binary + JSON state. No JavaScript
  runs on the server.
- Only interactive components ship JavaScript to the browser.
- Templates are declarative: HTML for structure, CSS for styling, TypeScript
  for behavior. They are always in **separate files**.
- There is no JSX, no CSS-in-JS, no template literals, no virtual DOM.

## The SSR Mental Model

```
BUILD TIME          SERVER RENDER         CLIENT HYDRATION
─────────────       ──────────────        ─────────────────
HTML + CSS + TS →   protocol.bin    →     Web Components
webui build         + JSON state          hydrate as islands
                    → rendered HTML
```

### Rules

1. **Every template binding must exist in the server state JSON.**
   If your template uses `{{title}}`, the server must provide
   `{ "title": "..." }`.

2. **Derived state belongs in the server or the template.** Use template
   expressions like `items.length` or `status == 'active'` for simple
   derivations. For complex values, compute on the server.

3. **The server is the source of truth for the initial render.** The client
   takes over after hydration for user interactions.

4. **Scriptless components are dormant, not dead.** Their bindings render on the
   server and contribute no initial bootstrap state. When the framework is
   loaded, compiler-owned hosts can activate for browser-applied state, parent
   property writes, or soft navigation. Events, lifecycle code, decorators, and
   imperative APIs need a same-named `.ts` or `.js` module.

5. **Hydration state is client-facing.** Initial `#webui-data.state` is
   projected to hydration keys for components reachable on the active route.
   This reduces CPU and bytes, but it is not a secrecy boundary. Never put
   credentials or private tokens in browser render state.

## Project Structure

```
my-app/
├── src/
│   ├── index.html              ← Entry template
│   ├── index.ts                ← Hydration entry point
│   ├── my-component/
│   │   ├── my-component.html   ← Component template
│   │   ├── my-component.css    ← Component styles (scoped)
│   │   └── my-component.ts     ← Component behavior
│   └── other-widget/
│       ├── other-widget.html
│       ├── other-widget.css
│       └── other-widget.ts
├── build-client.mjs           ← Application-owned browser bundle + manifest
├── data/
│   └── state.json              ← Server state for dev
├── dist/
│   ├── index.js                ← Browser entry/chunks
│   └── webui-projection.json   ← Build-time state projection sidecar
└── package.json
```

**Component discovery rules:**
- HTML files with a hyphen in the name are components (`my-card.html` → `<my-card>`)
- CSS files with the same name are auto-paired (`my-card.css`)
- A same-named TypeScript or JavaScript file opts the component into authored
  behavior (`my-card.ts`)
- With a validated projection manifest, only exact `@observable` / `@attr`
  fields opt into initial state hydration; without a manifest, state remains
  full
- Scriptless components retain compiler-owned browser templates but contribute
  no keys to initial bootstrap state
- Discovery is recursive through subdirectories

## The `<template>` Tag

The `<template shadowrootmode="open">` wrapper is **optional** in component
HTML files. The build tool auto-injects it when absent.

**Omit it** for most components (the framework wraps your content automatically):
```html
<!-- my-card.html -->
<h2>{{title}}</h2>
<p>{{description}}</p>
```

**Include it** when you need root host events on the shadow root itself:
```html
<!-- todo-app.html -->
<template shadowrootmode="open"
  @toggle-item="{onToggleItem(e)}"
  @delete-item="{onDeleteItem(e)}"
>
  <for each="item in items">
    <todo-item id="{{item.id}}"></todo-item>
  </for>
</template>
```

Root host events catch custom events bubbling up from child components.
This is the delegated event pattern for parent-child communication.

## Template Syntax

### Text binding

```html
<span>{{user.name}}</span>
<p>{{items.length}} items</p>
```

- `{{expr}}` - HTML-escaped output (safe for user input)
- `{{{expr}}}` - raw/unescaped output (only for trusted content)

### Conditionals

```html
<if condition="isLoggedIn">
  <p>Welcome back, {{username}}!</p>
</if>

<if condition="!hasItems">
  <p>No items found.</p>
</if>

<if condition="status == 'active'">
  <span class="badge">Active</span>
</if>
```

Supported operators: `==`, `!=`, `>`, `<`, `>=`, `<=`, `&&`, `||`, `!`

**Constraints:**
- Maximum 5 logical operators per expression
- Cannot mix `&&` and `||` in the same expression
- No parentheses for grouping
- No ternary operator (`? :`)

### Loops

```html
<for each="item in items">
  <div>{{item.name}} - {{item.price}}</div>
</for>
```

- The collection must be a JSON array
- Nested loops are supported; outer loop variables remain accessible
- Components inside loops do NOT inherit loop variables. Pass data via attributes:
  ```html
  <for each="contact in contacts">
    <contact-card name="{{contact.name}}" email="{{contact.email}}"></contact-card>
  </for>
  ```

### Attributes

```html
<!-- Dynamic attribute -->
<a href="{{url}}">{{linkText}}</a>

<!-- Boolean attribute (rendered when truthy, omitted when falsy) -->
<button ?disabled="{{isLoading}}">Submit</button>
<input type="checkbox" ?checked="{{isSelected}}" />

<!-- Boolean attributes accept the same expressions as <if condition="...">.
     Use comparisons against existing state instead of creating mirror observables. -->
<button ?disabled="{{currentIndex == 0}}">Prev</button>
<button ?disabled="{{currentIndex == items.length - 1}}">Next</button>
<option ?selected="{{item.id == selectedId}}">{{item.name}}</option>

<!-- Mixed static + dynamic -->
<img src="/img/{{user.avatar}}" alt="{{user.name}}" />

<!-- Complex/property binding -->
<my-widget :config="{{settings}}"></my-widget>
```

Property bindings use `:` to write directly to DOM properties. For
client-created component trees, initial property bindings are applied before a
child component's `connectedCallback` runs. Children can read parent-provided
values during setup, initialize their own fallback when a value is missing, and
still receive later parent updates through the live binding.

### Events (client-side only)

```html
<button @click="{handleClick()}">Click me</button>
<input @keydown="{onKeydown(e)}" />
<button @click="{selectItem(item.id, 'details', e)}">Select</button>
<div @mouseenter="{onHover()}" @mouseleave="{onLeave()}">Hover</div>
```

Event handler arguments can be `e`, dotted component or repeat-scope paths,
string/number/boolean/null literals, or a mix of those. Nested JavaScript
expressions are not parsed in templates.

### DOM references

```html
<input w-ref="searchInput" type="text" />
```

In the TypeScript class: `searchInput!: HTMLInputElement;`

### Routes

```html
<route path="/" component="app-shell">
  <route path="" component="home-page" exact />
  <route path="users" component="user-list" exact />
  <route path="users/:id" component="user-detail" exact />
</route>
```

**Path & matching:**
- Child paths are relative to parent (no leading `/`)
- Use `exact` on leaf routes (no children)
- Omit `exact` on parent routes that have `<outlet />`
- Path params: `:id` (required), `:query?` (optional), `*path` (catch-all)

**Attributes on `<route>`:**

| Attribute | Example | Description |
|-----------|---------|-------------|
| `path` | `"users/:id"` | URL path template (relative to parent) |
| `component` | `"user-detail"` | Component tag to mount |
| `exact` | (boolean) | Require exact path match (use on leaf routes) |
| `query` | `"action,to,subject"` | Allowlist of query params set as component attributes (deny-by-default) |
| `keep-alive` | (boolean) | Preserve DOM and local state across navigations |
| `cache-tags` | `"thread:{threadId},inbox"` | Cache tag templates - `{param}` resolved at render time |
| `invalidates` | `"inbox,sent,counts"` | Tags to auto-invalidate after mutation actions |
| `pending` | `"loading-skeleton"` | Component for loading UI during slow navigations (>150ms) |
| `error` | `"error-display"` | Component for error boundary on fetch failure |

All attributes are validated at build time. Referencing a non-existent `pending` or `error` component is a compile error.

**State flow:**
- Scriptless route components use compiler-owned hosts when the framework is
  loaded, so partial navigation applies only their template-root state
- `keep-alive` preserves DOM and local state. On reactivation, only param/query attrs are updated
- Route loaders: `static loader({ params, query, signal })` on component class - fetches custom data instead of server state. Runs pre-commit. Falls back to server state on failure
- Keep-alive + loader: DOM preserved, loader refreshes data on reactivation
- Route actions: `Router.start({ actions: true })` enables `static action({ formData, params, signal })` on component class - handles `<form method="post">`. Returns `{ invalidateTags?, state? }`. Auto-invalidates cache with merged tags

**Cache & preload:**
- Preload on hover: `Router.start({ preload: true })` - speculatively fetches on link hover
- Tagged cache: `Router.start({ cache: { staleTime, gcTime, maxEntries } })` - responses cached by path, tagged with `cacheTags`
- Cache/preload are optional runtime tiers; default `Router.start()` does not load the cache module
- `Router.invalidateTags(tags)` - evict cache entries by tag
- `Router.invalidate(path?)` - evict by path or all

**Server headers:**
- `X-WebUI-Inventory` header: hex bitmask of loaded templates - server skips re-sending

### Outlet

```html
<!-- Parent component template -->
<nav>...</nav>
<main><outlet /></main>
```

## Component Class

Every interactive component extends `WebUIElement`:

```typescript
import { WebUIElement, attr, observable } from '@microsoft/webui-framework';

export class MyComponent extends WebUIElement {
  // --- Decorators ---

  // @attr: reflects to/from HTML attribute (kebab-case)
  // String mode (default):
  @attr label = 'Default';

  // Boolean mode (present = true, absent = false):
  @attr({ mode: 'boolean' }) disabled = false;

  // @observable: reactive state, changes trigger DOM updates
  @observable count = 0;
  @observable items: Item[] = [];

  // Derived state: computed in event handlers, not as a getter
  @observable totalPrice = 0;

  private recalcTotal(): void {
    this.totalPrice = this.items.reduce((sum, i) => sum + i.price, 0);
  }

  // --- DOM refs (populated by w-ref="name" in template) ---
  inputEl!: HTMLInputElement;

  // --- Event handlers (referenced by @event in template) ---
  onSubmit(): void {
    const text = this.inputEl.value.trim();
    if (!text) return;
    this.items = [...this.items, { text, price: 0 }];
    this.inputEl.value = '';
  }

  onKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter') this.onSubmit();
  }

  // --- Custom events (child-to-parent communication) ---
  onItemDelete(e: CustomEvent<{ id: string }>): void {
    this.items = this.items.filter(i => i.id !== e.detail.id);
  }
}

// Register as custom element
MyComponent.define('my-component');
```

Components can omit the `.ts` file when server-rendered output is final. The
sibling module is the authored behavior boundary: without it, WebUI emits no
bootstrap state for that component, but retains its compiled template for later
browser rendering. Create a custom element for events, custom lifecycle code,
imperative methods, or JavaScript-owned state.

`@observable` and `@attr` are optional. Use them when TypeScript code reads or
mutates the value directly, or when the value is part of the component's public
API.

### Decorator reference

| Decorator | Purpose | SSR? | Triggers DOM update? |
|-----------|---------|------|---------------------|
| `@attr` | HTML attribute reflection | Yes; an existing SSR host attribute wins | Yes |
| `@attr({ mode: 'boolean' })` | Boolean attribute (present/absent) | Yes; host presence wins | Yes |
| `@observable` | Reactive state used by TypeScript code | Yes (from JSON state) | Yes |


### Component API

| Method/Property | Description |
|----------------|-------------|
| `this.$emit(name, detail?)` | Dispatch a CustomEvent that bubbles up |
| `this.$update()` | Force a reactive update cycle |
| `this.$flushUpdates()` | Synchronously flush pending updates |
| `static define(tagName)` | Register as a custom element |
| `defineComponentAssets(manifest)` | Define lazy component assets with `preload(tag)` and `create(tag)` |

### Emitting custom events

```typescript
// Child component
this.$emit('item-selected', { id: this.id, name: this.name });
```

```html
<!-- Parent template catches it -->
<child-component @item-selected="{onItemSelected(e)}"></child-component>
```

```typescript
// Parent handler
onItemSelected(e: CustomEvent): void {
  this.selectedId = e.detail.id;
}
```

### Dynamic Component Loading

Components like dialogs, overlays, and drawers are declared as routes but loaded
on demand, not during initial navigation. Declare them in the route tree so they
can be loaded dynamically:

```html
<route path="/" component="app-shell">
  <route path="" component="home-page" exact />
  <route path="users/:id" component="user-detail" exact />
  <!-- Available for dynamic loading, but only loaded when needed -->
  <route path="settings" component="settings-dialog" exact />
</route>
```

Then load on demand with `Router.ensureLoaded` before creating the element:

```typescript
// Fetches template + CSS from /_webui/templates before showing UI.
await Router.ensureLoaded('settings-dialog');
this.showSettings = true;

// Batch multiple in one request
await Router.ensureLoaded('modal-a', 'modal-b', 'drawer-c');
```

The component's template is **not** sent during initial SSR or partial
navigation. It has zero client cost until requested.

If a user navigates directly to `/settings` (deep link), the component
renders normally in the outlet. It works both ways.

Configure a custom endpoint if needed:

```typescript
Router.start({
  templateEndpoint: '/api/templates', // default: '/_webui/templates'
  loaders: { ... },
});
```

Every route intended for partial navigation must register a custom element.
Authored routes register eagerly or through `loaders`. Scriptless templates are
registered by the framework's compiler-owned host runtime. If a route remains
unregistered after template publication and loader resolution, the router
navigates the document to let the server render the component.

Without `@microsoft/webui-router`, prebuild static assets and load them from a
CDN or the app's static folder:

```bash
webui build ./src --out ./dist --plugin=webui \
  --emit-component-assets settings-dialog
```

Rust callers can set `BuildOptions::component_asset_roots`; rendered ESM files
are returned in `BuildResult::component_asset_files` and written by
`build_to_disk()`. Node callers use `componentAssetRoots` and receive flattened
`componentAssetFiles` (`[filename, content, ...]`) from `build()`.

`webui serve` accepts the same `--emit-component-assets` flag and validates each
root on every dev build. HTML and theme-token errors in lazily loaded components
fail the build instead of being missed because the component is outside the
initial route tree. The dev server serves `<tag>.webui.js` from memory and
rebuilds it on change under `--watch`. No separate `webui build`/`--out` step is
needed during development.

```typescript
import { settingsAssets } from './lazy-assets.js';

async onOpenSettings(): Promise<void> {
  settingsAssets.preload('settings-dialog');
  this.panelSlot.replaceChildren(await settingsAssets.create('settings-dialog'));
}
```

```typescript
// lazy-assets.ts
import { defineComponentAssets } from '@microsoft/webui-framework/component-asset.js';

export const settingsAssets = defineComponentAssets({
  'settings-dialog': {
    asset: '/settings-dialog.webui.js',
    module: () => import('./settings-dialog/settings-dialog.js'),
    data: async () => await (await fetch('/settings-dialog-data.json')).json(),
  },
});
```

`defineComponentAssets()` uses the current page CSP nonce when it needs to append
CSS module importmaps. The `.webui.js` asset is a browser-native ESM module that
carries the component template and style payload in one request. Concurrent calls
for the same URL share one in-flight request, and CSS module styles are deduped.
The manifest helper lets the shell start the template asset, JS chunk, and data
fetch in parallel as soon as the user expresses intent via `preload(tag)`;
`create(tag)` waits for only the template asset and JS module by default, then
creates the element. Use
`create(tag, { awaitData: true, dataTimeoutMs: 150 })` only when a component
must wait briefly for state before mounting.

## Component CSS

CSS is scoped per component via Shadow DOM. No CSS-in-JS.

```css
/* my-component.css */
:host {
  display: block;
  padding: 1rem;
}

:host([disabled]) {
  opacity: 0.5;
  pointer-events: none;
}

:host([variant="primary"]) {
  background: var(--colorBrandBackground);
}

/* Internal elements */
.header { font-weight: bold; }
.content { padding: 0.5rem; }
```

**Rules:**
- `:host` styles the component root element
- `:host([attr])` styles based on attribute presence/value
- Internal selectors are scoped to the shadow root
- Use CSS custom properties (`var(--name)`) for theming. Nested fallbacks like
  `var(--primary, var(--fallback))` are also discovered as tokens.
- Malformed CSS fails the build, including unterminated `var()` calls, comments,
  strings, and unmatched delimiters.
- No styles leak in or out

## Entry Template

```html
<!DOCTYPE html>
<html lang="en" dir="{{textdirection}}">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>{{title}}</title>
</head>
<body>
  <app-shell></app-shell>
  <script type="module" src="/index.js"></script>
</body>
</html>
```

## Hydration Entry Point

```typescript
// index.ts
import { WebUIElement } from '@microsoft/webui-framework';

// Import components to register them as custom elements.
// Registration triggers hydration automatically.
import './app-shell/app-shell.js';
import './user-card/user-card.js';

// Optional: listen for hydration completion
window.addEventListener('webui:hydration-complete', () => {
  const total = performance.getEntriesByName('webui:hydrate:total', 'measure')[0];
  console.log(`Hydration: ${total?.duration.toFixed(1)}ms`);
});
```

## Build-Time State Projection

Rust never analyzes JavaScript or TypeScript. Bundle browser code first and emit
projection metadata from the same resolved graph:

```bash
npm install -D esbuild typescript
```

```javascript
// build-client.mjs
import * as esbuild from 'esbuild';
import { esbuildProjection } from '@microsoft/webui/projection.js';

await esbuild.build({
  entryPoints: ['src/index.ts'],
  outdir: 'dist',
  bundle: true,
  splitting: true,
  format: 'esm',
  plugins: [esbuildProjection()],
});
```

```bash
node build-client.mjs
webui build ./src --out ./dist --plugin=webui \
  --projection-manifest ./dist/webui-projection.json
```

Rules:

- `@microsoft/webui/projection.js` is build-only. `esbuild` and `typescript` are
  optional peers and are not imported by the root `@microsoft/webui` runtime.
- The compiler contract is bundler-neutral; the package currently includes the
  supported esbuild adapter.
- esbuild runs once. The adapter observes that run and writes
  `webui-projection.json` atomically after successful output.
- No manifest means full state and no JavaScript inference.
- Any supplied manifest enables strict coverage. Every scripted component in
  the protocol, including components discovered through `--components`, must
  have exactly one entry or the build fails with `PROJ-B001`.
- Code-split and external bundles remain application-owned. Build external
  shared controls separately and repeat `--projection-manifest` for each
  fragment.
- The manifest uses JavaScript property names for `@observable` and `@attr`.
  Existing SSR host attributes take precedence over projected `@attr` values.
- WebUI validates input/output hashes and embeds compact metadata in
  `protocol.bin`. Runtime handlers never open the manifest.

## Hydration with Router

```typescript
// index.ts
import { WebUIElement } from '@microsoft/webui-framework';
import { Router } from '@microsoft/webui-router';
import './app-shell/app-shell.js';

// Start router after components are registered
Router.start({
  loaders: {
    'home-page': () => import('./pages/home-page.js'),
    'user-list': () => import('./pages/user-list.js'),
    'user-detail': () => import('./pages/user-detail.js'),
  },
});
```

### View Transitions

The router wraps every client-side navigation in `document.startViewTransition()`
automatically. **Do not** wrap `Router.navigate()` in your own `startViewTransition()`
— that would double-transition.

While active, the router installs a nonce-bearing
`@view-transition { navigation: none; }` override. Automatic cross-document
transitions would conflict with intercepted routes that fall back to SSR
document requests. Explicit client-side transitions still use
`document.startViewTransition()`, and `Router.destroy()` removes the override.

To customize the animation, assign `view-transition-name` to elements in your CSS
and target them with `::view-transition-old()` / `::view-transition-new()`:

```css
.content-area { view-transition-name: content; }

::view-transition-old(content) { animation: fade-out 100ms ease-out; }
::view-transition-new(content) { animation: fade-in 150ms ease-in; }
```

The router awaits `updateCallbackDone` (not `.finished`) so rapid navigations
supersede each other without queuing.

## CLI Commands

### Build

```bash
webui build ./src --out ./dist --plugin=webui
```

| Flag | Default | Description |
|------|---------|-------------|
| `APP` (positional) | `.` | App source directory |
| `--out <DIR>` | *required* | Output directory |
| `--entry <FILE>` | `index.html` | Entry HTML file |
| `--css <MODE>` | `link` | `link`, `style`, or `module` |
| `--dom <MODE>` | `shadow` | `shadow` or `light` |
| `--plugin <NAME>` | none | Plugin identifier (e.g. `webui`) |
| `--components <PACKAGE>` | none | Extra component sources (repeatable) |
| `--projection-manifest <PATH>` | none | Bundler projection fragment (repeatable); requires `--plugin=webui` |
| `--emit-component-assets <TAGS>` | none | Comma-separated root component tags emitted as static `.webui.js` ESM assets in `--out` |
| `--theme <PACKAGE>` | none | Design token theme to validate against (see below) |
| `--asset-file-name-template <TEMPLATE>` | `[name].[ext]` | Emitted asset filename template for Link-mode CSS files and static component assets. Tokens: `[name]`, `[hash]`, `[ext]` |
| `--css-public-base <BASE>` | none | Public URL/path prefix for Link-mode CSS hrefs |
| `--legal-comments <MODE>` | `inline` | `inline` preserves legal CSS comments, `none` strips all comments |
| `--format <FORMAT>` | `human` | `human` (colorized) or `json` (machine-readable diagnostics on stdout) |

With `--css module`, WebUI appends
`shadowrootadoptedstylesheets="<component-name>"` to component `<template>`
wrappers when needed. If you author the wrapper yourself for root events, keep
your attributes there; WebUI preserves them for client/plugin templates.

For framework apps that bundle browser code, bundle your source browser entry
directly. Import `@microsoft/webui-framework` from authored component modules.
An app that stays static after SSR needs no framework browser runtime. Import
the framework once when scriptless components need browser state or soft
navigation.

For exact state projection, run the browser bundler first and pass its completed
manifest to `webui build`. Repeat `--projection-manifest` for separately built
external component bundles. Omitting the flag preserves full state.

For CDN/browser caching in `link` mode, prefer:

```bash
webui build ./src --out ./dist \
  --asset-file-name-template "[name]-[hash].[ext]" \
  --css-public-base "https://cdn.example.com/assets"
```

`[hash]` is the component CSS file's SHA-256 content hash truncated to 8 hex
characters. The CSS files are still written to `--out`; `--css-public-base`
changes only the href compiled into `protocol.bin` and emitted in stylesheet
`<link>` tags.

For lazy components without `@microsoft/webui-router`, emit static component
assets:

```bash
webui build ./src --out ./dist --plugin=webui \
  --emit-component-assets mail-thread,compose-page
```

This writes `mail-thread.webui.js`. Requested roots stay outside initial SSR
unless the entry template also references them. The asset is standard ESM that
carries the component template and styles. Load it with
`defineComponentAssets()` before mounting the component. Use
`--asset-file-name-template "[name]-[hash].[ext]"` for CDN-cacheable filenames.
Do not reference the lazy component tag from an SSR-reachable template unless you
intentionally want it eligible for initial SSR.

HTML comments are stripped at build time and bindings inside them are ignored.
CSS comments are stripped except legal comments and `<style>` signal fragments.
Legal CSS comments containing `@license` or `@preserve`, or starting with `/*!`
or `//!`, are preserved only when `--legal-comments inline` is active.
Malformed HTML or CSS fails `webui build`; escape literal `<` as `&lt;` and
close all tags, comments, declarations, `var()` calls, and CSS delimiters.

### Serve (dev server)

```bash
webui serve ./src --state ./data/state.json --plugin=webui --watch
```

| Flag | Default | Description |
|------|---------|-------------|
| `APP` (positional) | `.` | App source directory |
| `--state <FILE>` | *required* | JSON state file |
| `--port <PORT>` | `3000` | Server port |
| `--watch` | false | Enable live reload |
| `--servedir <DIR>` | none | Static asset directory |
| `--entry <FILE>` | `index.html` | Entry HTML file |
| `--css <MODE>` | `link` | `link`, `style`, or `module` |
| `--dom <MODE>` | `shadow` | `shadow` or `light` |
| `--plugin <NAME>` | none | Plugin identifier (e.g. `webui`) |
| `--components <PACKAGE>` | none | Extra component sources (repeatable) |
| `--projection-manifest <PATH>` | none | Bundler projection fragment (repeatable); watched explicitly with `--watch` |
| `--api-port <PORT>` | none | Proxy route requests to API server |
| `--theme <PACKAGE>` | none | Design token theme; missing unresolved tokens fail the build (see below) |
| `--asset-file-name-template <TEMPLATE>` | `[name].[ext]` | Emitted asset filename template |
| `--css-public-base <BASE>` | none | Public URL/path prefix for Link-mode CSS hrefs |
| `--legal-comments <MODE>` | `inline` | `inline` preserves legal CSS comments, `none` strips all comments |
| `--format <FORMAT>` | `human` | `human` (colorized) or `json` (machine-readable diagnostics on stdout) |

### Inspect

```bash
webui inspect ./dist/protocol.bin
webui inspect ./dist/protocol.bin | jq '.fragments | keys'
```

## Build Diagnostics & Error Output

Authoring mistakes fail `webui build` with a structured, actionable diagnostic
(never a stack trace). Each one has a **stable error code**, the source
location, the offending snippet, and a `help:` fix:

```
✘ error: invalid <for> each expression [invalid-for-each]
  --> index.html:67:5
    each="person inpeople"
  help: use the form each="item in collection", e.g. each="todo in todos"
```

When a mistake looks like a typo, `help:` suggests the intended name — a
misspelled directive attribute (`eahc` → `each`) or an unregistered
custom-element tag that closely matches a registered component **in the same
namespace** (`<mp-buton>` → `<mp-button>`). A different-namespace tag (e.g. a
third-party `<md-button>`) passes through as a genuine custom element.

### Machine-readable output (`--format json`)

For editors, CI, and AI tooling, pass the global `--format json` flag. Each
error is emitted as a single JSON object on **stdout** (no ANSI), so it can be
parsed directly instead of scraping terminal text:

```bash
webui build ./src --out ./dist --format json
```

```json
{
  "severity": "error",
  "code": "invalid-for-each",
  "message": "invalid <for> each expression",
  "file": "index.html",
  "line": 67,
  "column": 5,
  "snippet": "each=\"person inpeople\"",
  "help": "use the form each=\"item in collection\", e.g. each=\"todo in todos\"",
  "chain": ["Build failed", "Failed to parse index.html", "..."]
}
```

Branch on the stable `code`, not the human-readable `message`. Fields that don't
apply to a given error are `null`.

### Error codes

`invalid-for-each`, `invalid-for-identifier`, `missing-for-each`,
`invalid-if-condition`, `missing-if-condition`, `unknown-component`,
`invalid-event-handler`, `invalid-w-ref`, `missing-theme-token`,
`unclosed-html-tag`,
`malformed-html-tag`, `unexpected-closing-tag`, `unterminated-html-comment`,
`unterminated-html-declaration`, `excessive-nesting`, `recursive-template`,
`invalid-css`, `PROJ-P001`, `PROJ-P002`, `PROJ-B001`, `PROJ-B002`,
`PROJ-M001`, `PROJ-M003`, `PROJ-M004`, `PROJ-M006`, `PROJ-M007`,
`PROJ-M009`, `PROJ-S001`, `PROJ-S003`, `PROJ-S004`.

### Exit codes

Following `sysexits.h`: `0` success, `2` argument/usage error, `65`
template/authoring error, `66` missing input (app folder, `--state`,
`--servedir`, or entry file), `69` port already in use, `74` I/O error, `1`
otherwise.

## `--components` - External Component Sources

The `--components` flag discovers components from npm packages or local
directories outside your app folder. Repeatable.

**What it accepts:**

- **npm package name** - `@scope/my-widgets` or `my-widget`
- **Scoped prefix** - `@scope` (discovers ALL sub-packages under `node_modules/@scope/`)
- **Local path** - `./shared/components` or `/libs/ui-kit`

Values starting with `.`, `/`, `\`, or a drive letter are treated as local
paths. Everything else is treated as an npm package name.

**npm package requirements:**

The package's `package.json` must have:

```json
{
  "name": "@scope/my-button",
  "customElements": "./custom-elements.json",
  "exports": {
    "./template-webui.html": "./dist/template-webui.html",
    "./styles.css": "./dist/styles.css"
  }
}
```

| Field | Required | Purpose |
|-------|----------|---------|
| `exports["./template-webui.html"]` | Yes | Component HTML template |
| `exports["./styles.css"]` | No | Component CSS |
| `customElements` | Yes | Path to Custom Elements Manifest (provides tag name) |

Packages with a root JavaScript entry (`exports["."]`, `main`, `module`, or
`browser`) are authored custom-element packages. Packages with only WebUI
template/style exports are compiler-owned template libraries. Their dynamic
templates render on the server and can activate in the browser when the
framework runtime is loaded.

**Local path scanning** works like app directory scanning: HTML files with
hyphenated names are registered as components, matching CSS files are
auto-paired. A sibling `.ts` or `.js` file marks the component as authored and
interactive.

**Caching:** npm results are cached at `~/.webui/cache/components/` and
invalidated when `package.json` changes. Local paths are always re-scanned.

```bash
# Single scoped package
webui build ./src --out ./dist --components @reactive-ui/button

# All packages under a scope
webui build ./src --out ./dist --components @reactive-ui

# Local shared library
webui build ./src --out ./dist --components ./shared/components

# Multiple sources
webui build ./src --out ./dist \
  --components @reactive-ui \
  --components ./shared/components
```

## `--theme` - Design Token Themes

The `--theme` flag loads a token JSON file. On `webui build`, it validates that
each unresolved CSS token discovered by the parser exists in every theme. On
`webui serve`, it performs the same validation and injects resolved CSS custom
property declarations into the render state.

**What it accepts:**

- **Local JSON file** - `./themes/dark.json`
- **npm package** - `@my-org/brand-tokens` (looks for `tokens.json` inside the package)
- **npm package with subpath** - `@my-org/brand-tokens/custom.json`

**How it works:**

1. Loads the JSON file (multi-theme or flat single-theme format)
2. Filters tokens to only those actually used in your CSS (`var(--name)`,
   including nested `var()` fallbacks)
3. Expands present transitive `var()` references; theme internals are trusted, so
   missing/cyclic references inside theme values are left to browser CSS
   semantics
4. Fails with `missing-theme-token` when any required token is absent from a
   theme. `var(--a, var(--b, var(--c)))` requires `a`, `b`, and `c` unless a
   token is defined by local/ancestor CSS. A `var()` usage with a literal
   fallback (e.g. `var(--brand, #000)`) is exempt — the token is still hoisted
   for runtime resolution but its absence does not fail the build, unless the
   same token is also used without a fallback.
5. Generates CSS declaration strings per theme
6. Injects into SSR state as `state.tokens.light`, `state.tokens.dark`, etc.
   These render-only token strings are omitted from the emitted client state.

A token used only with a literal `var()` fallback and absent from every theme
(e.g. a misspelled `var(--colr-brand, #000)`) is reported as a non-fatal
`unthemed-token` **warning** on `BuildResult.warnings` (a `Vec<Diagnostic>`,
also printed by `webui build` and `webui serve`) instead of failing the build —
a typo safety net. Warnings are warning-severity `Diagnostic`s, so they render
with the same layout as `missing-theme-token` errors: both carry the source
location (`my-card.css:2:10` + the CSS line) and a `did you mean --…?`
suggestion computed by Levenshtein edit distance against the theme's tokens. The
dev server frames each error/warning with blank lines so consecutive advisories
stay readable.

In `webui serve --watch`, rebuild failures are retained in dev-server state:
the terminal and live-reload SSE report the error, and a browser refresh returns
the latest rebuild error instead of stale HTML while keeping the live-reload
connection active until the next clean rebuild.

**Multi-theme format:**
```json
{
  "themes": {
    "light": {
      "surface-page": "#ffffff",
      "text-primary": "#111827"
    },
    "dark": {
      "surface-page": "#171717",
      "text-primary": "#fafafa"
    }
  }
}
```

**Flat format** (single theme, treated as `"default"`):
```json
{
  "surface-page": "#ffffff",
  "text-primary": "#111827"
}
```

**Template usage** - wrap dynamic CSS fragments in a CSS block comment:
```html
<style>
  :root {
    /*{{{tokens.light}}}*/
  }
</style>
```

The handler resolves `tokens.light` from the state, outputting:
```css
:root {
  --surface-page: #ffffff;
  --text-primary: #111827;
}
```

## State JSON

```json
{
  "title": "My App",
  "user": { "name": "Alice", "role": "admin" },
  "items": [
    { "id": "1", "label": "First", "done": false },
    { "id": "2", "label": "Second", "done": true }
  ],
  "isAdmin": true,
  "showBanner": false
}
```

**Path resolution:** `title`, `user.name`, `items.0.label`, `items.length`

**Missing paths:** text bindings render empty, `<if>` evaluates to false. No error.

## Truthiness in Conditions

| Value | Truthy? |
|-------|---------|
| `true` | Yes |
| `false` | No |
| `0` | No |
| Non-zero number | Yes |
| `""` (empty string) | No |
| `"false"` (string!) | **Yes** (non-empty string) |
| `null` / missing key | No |

**Never use string `"false"` for boolean state. Use real booleans.**

## Things You CANNOT Do

1. **No ternary in templates.** `{{x ? 'yes' : 'no'}}` does not work.
   Use `<if>` blocks or boolean attributes instead.

2. **No function calls in bindings.** `{{formatDate(item.date)}}` does not
   work. Compute the value on the server or in an event handler.

3. **No mixed `&&` and `||`.** `<if condition="a && b || c">` is invalid.
   Split into nested `<if>` blocks.

4. **No parentheses in conditions.** `<if condition="(a && b) || c">` is
   invalid.

5. **No JavaScript in HTML templates.** Templates are compiled to binary.
   Logic goes in the TypeScript class file, not the template.

6. **No JavaScript in CSS.** CSS is plain CSS in `.css` files. Use CSS
   custom properties for dynamic values.

7. **No computed getters for SSR state.** If a value appears in the
   template, it must be in the server state JSON. Use `@observable`
   only when event handlers or other TypeScript code read or change it.

8. **Components inside `<for>` loops do NOT inherit loop variables.**
   Pass data explicitly via attributes.

9. **No `import` or `require` in templates.** Components are discovered
   by file naming convention, not imports.

10. **No `this.querySelector()` for reactive state.** Use `@observable` for
    state your TypeScript changes, and use template bindings for DOM output.
    Use `w-ref` only for imperative DOM access (focus, scroll, etc.).

11. **No `@observable` writes before `super.connectedCallback()`.** During SSR
    hydration the server-rendered DOM is trusted and not re-rendered, so a value
    set in a field initializer, the `constructor`, or before
    `super.connectedCallback()` cannot reach the DOM — the write is dropped and
    the runtime logs a `[WebUI] Hydration mismatch` warning. If the value must
    appear in the first render, put it in the SSR state JSON; otherwise assign it
    after `super.connectedCallback()`. The warning is development-only — it is
    stripped from production bundles via the `__WEBUI_DEV__` compile-time flag
    (`webui-press build` sets `__WEBUI_DEV__=false` automatically; self-bundled
    apps add the define for production).

## Common Patterns

### Toggle visibility

```html
<button @click="{togglePanel()}">Toggle</button>
<if condition="isPanelOpen">
  <div class="panel">Panel content</div>
</if>
```

```typescript
@observable isPanelOpen = false;
togglePanel(): void { this.isPanelOpen = !this.isPanelOpen; }
```

### List with add/remove

```html
<input w-ref="input" @keydown="{onKey(e)}" />
<for each="item in items">
  <div>
    {{item.text}}
    <button @click="{remove(item.id)}">×</button>
  </div>
</for>
```

`remove(item.id)` receives the current loop item's id. The framework captures
the active repeat scope during hydration and resolves the path when the click
event fires.

### Boolean attribute styling

```html
<button ?data-active="{{isActive}}" @click="{toggle()}">
  {{label}}
</button>
```

```css
button[data-active] { background: blue; color: white; }
button:not([data-active]) { background: transparent; }
```

### Lazy-loaded dialog

Declare the component as a route (so it's compiled), then load dynamically:

```html
<!-- index.html - settings-dialog is available but not navigated to -->
<route path="/" component="app-shell">
  <route path="" component="home-page" exact />
  <route path="settings" component="settings-dialog" exact />
</route>
```

```typescript
// Shell component - load template + CSS on demand
async onOpenSettings(): Promise<void> {
  await Router.ensureLoaded('settings-dialog');
  await import('./settings-dialog/settings-dialog.js');
  this.showSettings = true;
}
```

```html
<!-- Shell template - create the element dynamically -->
<if condition="showSettings">
  <settings-dialog @close="{onCloseSettings()}"></settings-dialog>
</if>
```

### Derived state (prefer template expressions over shadow observables)

Anywhere an expression works (`<if condition="...">`, `?boolAttr="{{...}}"`, text
bindings via path lookups), compare existing state directly. Do not introduce
extra observables that just mirror a comparison or sentinel of other state, and
do not bake per-item flags like `isCurrent` / `isDisabled` into the SSR JSON
when a single comparison can derive them.

```html
<!-- DO: derive in the template -->
<if condition="items.length">
  <span>{{items.length}} items</span>
</if>

<button ?disabled="{{currentIndex == 0}}">Prev</button>
<button ?disabled="{{currentIndex == totalItems}}">Next</button>

<for each="app in apps">
  <option ?selected="{{app.slug == currentApp.slug}}">{{app.name}}</option>
</for>

<!-- DON'T: shadow observables that mirror a comparison -->
<!-- @observable hasItems = false;     // mirrors items.length > 0 -->
<!-- @observable prevDisabled = true;  // mirrors currentIndex == 0 -->
<!-- @observable nextDisabled = false; // mirrors currentIndex == totalItems -->

<!-- DON'T: per-item flags in JSON state that mirror a comparison -->
<!-- { "apps": [{ "slug": "...", "isCurrent": true }, ...] }
     Just compare against the selected slug/id in the template. -->
```

Loop variables (e.g. `app`) compose with outer component state (e.g.
`currentApp`) inside the same expression, so per-iteration flags are almost
never needed.

Text bindings only do path lookups — they can't do arithmetic. If you need
`{{currentIndex + 1}}` for a 1-based display, that's a legitimate `@observable`
(or precomputed in the SSR state).

### Route-scoped state

Each route handler should return only the state that route's component needs:

```json
// GET /inbox -> only inbox data
{ "threads": [...], "selectedFolder": "inbox" }

// GET /settings -> only settings data
{ "theme": "dark", "language": "en" }
```

## package.json

```json
{
  "scripts": {
    "build:client": "node build-client.mjs",
    "build:protocol": "webui build ./src --out ./dist --plugin=webui --projection-manifest ./dist/webui-projection.json",
    "build": "npm run build:client && npm run build:protocol",
    "dev:server": "webui serve ./src --state ./data/state.json --plugin=webui --projection-manifest ./dist/webui-projection.json --watch"
  },
  "dependencies": {
    "@microsoft/webui": "latest",
    "@microsoft/webui-framework": "latest"
  }
}
```

Add `@microsoft/webui-router` if using client-side navigation.
Run the client bundler in watch mode alongside `dev:server`; the dev server
rebuilds when the adapter atomically replaces the manifest.

## Language Integration (Server Side)

WebUI renders from **any** backend. The server loads and prepares
`protocol.bin` once, then renders with JSON state per request. Projection
manifests are consumed only while producing `protocol.bin`; runtime rendering
APIs are unchanged.

### Rust

```rust
let protocol = PreparedProtocol::from_protobuf(&fs::read("dist/protocol.bin")?)?;
let state = json!({ "title": "Home", "items": items_vec });
let handler = WebUIHandler::new();
handler.handle(protocol.protocol(), &state, &options, &mut writer)?;
```

**Streaming SSR (production).** Use `webui::streaming::StreamingWriter::new_pooled(tx, chunk_pool)` with a process-wide `ChunkPool` for bounded backpressure + zero per-flush allocation. Configure `.with_flush_timeout(Duration::from_secs(30))` to bound slow-loris DoS. Use `RenderOptions::with_head_inject(html)` / `with_body_inject(html)` for per-request HTML splicing at parser-synthesized `head_end` / `body_end` boundaries (no byte-scanner, cannot mis-fire on literals in comments / srcdoc). `HandlerError::ClientDisconnected` and `StreamTimeout` are returned from both `write()` and `end()` for telemetry. Pre-escape untrusted inject content with `webui_handler::encode_safe`.

### Node.js

```javascript
import { build, render } from '@microsoft/webui';

const result = build({
  appDir: './src',
  plugin: 'webui',
  projectionManifests: ['./dist/webui-projection.json'],
});

// Keep this Buffer for the server lifetime. Reusing its identity reuses the
// native prepared protocol.
const protocol = result.protocol;
const html = render(protocol, state, {
  entry: 'index.html',
  requestPath: req.url,
  plugin: 'webui',
});
```

### WebAssembly

Use the split WASM bundles when rendering or parsing in the browser:

```javascript
import initHandler, { PreparedProtocol } from './wasm/handler/webui_wasm_handler.js';
await initHandler();
const protocolBytes = new Uint8Array(await (await fetch('/protocol.bin')).arrayBuffer());
const protocol = new PreparedProtocol(protocolBytes);
const html = protocol.renderJson(
  JSON.stringify(state),
  { entry: 'index.html', requestPath: '/', plugin: 'webui' },
);
```

```javascript
import initParser, { build_protocol } from './wasm/parser/webui_wasm_parser.js';
await initParser();
const protocolBytes = build_protocol(
  { 'index.html': '<h1>{{title}}</h1>' },
  'index.html',
  [projectionManifest],
);
```

Use `wasm/all/webui_wasm_all.js` when both parser and handler exports are needed in one module, such as a playground.

### Python (FFI)

```python
ptr = lib.webui_render(html_bytes, json_bytes)
result = ctypes.cast(ptr, c_char_p).value.decode("utf-8")
lib.webui_free(ptr)
```

For repeated FFI rendering, use `webui_protocol_create` once and pass that
handle to `webui_handler_render` on every request.

### Go (cgo)

```go
ptr := C.webui_render(cHTML, cJSON)
defer C.webui_free(ptr)
result := C.GoString(ptr)
```

### C# (.NET package)

```csharp
using var protocol = new PreparedProtocol(File.ReadAllBytes("dist/protocol.bin"));
using var handler = new WebUIHandler("webui");
string result = handler.Render(protocol, dataJson, "index.html", requestPath);
```

### Server Template Endpoint

For `Router.ensureLoaded()`, expose `GET /_webui/templates?t=tag1,tag2`:

```rust
let result = route_handler::render_component_templates(&protocol, &tags, &inv);
```

```javascript
// Node native addon
const result = renderComponentTemplates(protocolBuf, JSON.stringify(tags), invHex);

// @microsoft/webui npm package
import { renderComponentTemplates } from '@microsoft/webui';
const result = renderComponentTemplates(protocolBuf, ['settings-dialog'], invHex);
```

The JSON response contains component-tag-keyed `templates`, matching
`templateFunctions`, `templateStyles` for CSS module importmaps, and `inventory`
with the updated component bitmask.
