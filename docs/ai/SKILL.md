---
layout: page
name: webui-reference
description: Authoritative WebUI framework reference for generating correct application code - template-first authoring rules, template syntax, styling, interactivity, routing, state JSON, and anti-patterns.
---

# WebUI Framework - AI Reference

> **Single-page reference for LLMs.** Everything an AI coding assistant needs to
> generate correct WebUI code. Read the Rules first - they are the constraints
> that most often get violated. Deep-dive links are indexed at the bottom.
>
> Install this reference into your agent with
> `npx skills add microsoft/webui --skill webui-reference` - see
> [AI Coding Agents](/guide/installation#ai-coding-agents).

## Rules

These four rules decide almost every authoring question. When in doubt, re-read
them before writing code.

### 1. The template is the UI

All UI structure lives in `.html` template files. There is no other way to
create UI.

- Never `document.createElement()`, `innerHTML`, `insertAdjacentHTML`,
  `appendChild`, `cloneNode`, or `new DOMParser()` to build UI.
- Show and hide with `<if>`. Repeat with `<for>`. Swap regions with `<outlet>`.
- If the markup you need does not exist yet, add it to the template and gate it
  with `<if>` - do not build it from JavaScript.
- **The single exception** is mounting a lazily loaded component, which has no
  template representation until it is fetched. See
  [Lazy component mounting](#lazy-component-mounting).

### 2. CSS owns all styling and animation

All styling lives in `.css` files. All animation is declarative CSS.

- Never `el.style.color = ...`, `classList.add/remove/toggle`,
  `setAttribute('style', ...)`, `adoptedStyleSheets`, or injected `<style>` tags.
- To style from state, bind an attribute in the template and select on it in CSS:
  `?data-active="{{isActive}}"` plus `[data-active] { ... }`.
- Animate with `transition`, `@keyframes`, `@starting-style`, and view
  transitions. Never `element.animate()`, `requestAnimationFrame` tweening,
  `setInterval` timers, or a JavaScript animation library.

### 3. JavaScript is opt-in, and only for interactivity

A component needs **no** `.ts` file at all unless something interactive happens
in it. Scriptless components still render bindings, `<if>`, and `<for>` on the
server, and still activate for browser state and soft navigation.

- `WebUIElement`, `@observable`, and `@attr` are **optional**. Add them only
  when JavaScript actually reads or writes the value, or when the value is part
  of the component's public API that another module sets.
- A value that only appears in the template belongs in the server state JSON,
  not in an `@observable`.
- JavaScript is for event handlers, network calls, focus management, and
  imperative browser APIs. Nothing else.

### 4. Use the web platform

Prefer a built-in HTML element or modern CSS feature over a hand-built one.

- `<dialog>` over a div-based modal. `popover` over a JS dropdown.
  `<details>` over a JS accordion.
- `:has()`, `@container`, `@starting-style`, `field-sizing`, `color-mix()`,
  `light-dark()`, `content-visibility`, `inert`, `@layer`.
- Native form validation, `<input type="...">` variants, `loading="lazy"`.
- No UI libraries, no CSS frameworks, no polyfills for baseline features.

## What to reach for

| You need | Use this | Not this |
|---|---|---|
| Show / hide a region | `<if condition="...">` | `el.hidden`, `style.display` |
| Render a list | `<for each="x in xs">` | `createElement` in a loop |
| Style from state | `?data-x="{{expr}}"` + CSS attribute selector | `classList.toggle` |
| Toggle a class-like variant | `?data-variant="{{mode == 'compact'}}"` | `className = ...` |
| Modal | `<dialog>` + `showModal()` | div overlay + z-index juggling |
| Dropdown / tooltip / menu | `popover` + `:popover-open` | JS positioning library |
| Accordion / disclosure | `<details><summary>` | click handler + height math |
| Tabs | radio inputs or `?data-selected` + CSS | JS show/hide of panels |
| Enter / exit animation | `@starting-style` + `transition-behavior: allow-discrete` | `element.animate()` |
| Looping animation | `@keyframes` | `requestAnimationFrame` loop |
| Page transition | `view-transition-name` + `::view-transition-*` | manual fade with JS |
| Responsive to container | `@container` | `ResizeObserver` + inline styles |
| Style a parent from a child | `:has()` | JS class propagation |
| Theme values | CSS custom properties + `--theme` | JS theme objects |
| Derived boolean | `<if condition="items.length">` | `@observable hasItems` |
| Text that never changes on the client | server state JSON | `@observable` |
| Focus / scroll / measure | `w-ref="{el}"` + a `.ts` file | `querySelector` |

## Mental model

```
BUILD TIME          SERVER RENDER         CLIENT HYDRATION
---------------     ---------------       -----------------
HTML + CSS + TS ->  protocol.bin    ->    Web Components
webui build         + JSON state          hydrate as islands
                    -> rendered HTML
```

WebUI is a **language-agnostic server-side rendering framework**. Templates
compile to a binary Protocol Buffer at build time. At runtime any backend
(Rust, Node, Go, C#, Python) supplies JSON state and produces HTML. On the
client, interactive components hydrate as islands.

1. **Every template binding must exist in the server state JSON.** If the
   template uses `{{title}}`, the server must provide `{ "title": "..." }`.
   Missing paths render empty and `<if>` evaluates false. No error is raised.
2. **Derived state belongs in the template or the server.** Use expressions like
   `items.length` or `status == 'active'`. Compute complex values server-side.
3. **The server is the source of truth for the initial render.** The client
   takes over after hydration for user interactions.
4. **Scriptless components are dormant, not dead.** Their bindings render on the
   server and contribute no initial bootstrap state. Compiler-owned hosts can
   still activate for browser-applied state, parent property writes, or soft
   navigation. Events and lifecycle code need a same-named `.ts` or `.js`.
5. **Hydration state is client-facing.** It reduces CPU and bytes but is not a
   secrecy boundary. Never put credentials or private tokens in render state.

## Project structure

```
my-app/
|- src/
|  |- index.html              <- Entry template
|  |- index.ts                <- Hydration entry point
|  |- my-component/
|  |  |- my-component.html    <- Component template
|  |  |- my-component.css     <- Component styles (scoped)
|  |  \- my-component.ts      <- Optional: only if interactive
|  \- static-widget/
|     |- static-widget.html   <- Scriptless: no .ts needed
|     \- static-widget.css
|- data/state.json            <- Server state for dev
\- dist/
```

**Component discovery:**

- HTML files with a hyphen in the name are components
  (`my-card.html` -> `<my-card>`).
- CSS files with the same name are auto-paired.
- A same-named `.ts` or `.js` file opts the component into authored behavior.
  Most components should not have one.
- Discovery is recursive through subdirectories.

## Template syntax

### HTML structure

Write browser-valid HTML nesting. Native void tags are matched
case-insensitively, and direct `<col>` / `<tr>` runs receive the browser-implied
`<colgroup>` / `<tbody>` in compiled client metadata. When an `<if>` or `<for>`
controls table columns or rows, author the corresponding `<colgroup>` or
`<tbody>` explicitly so both SSR hydration markers stay in the same browser
parsing context.

### Text binding

```html
<span>{{user.name}}</span>
<p>{{items.length}} items</p>
```

- `{{expr}}` - HTML-escaped output (safe for user input)
- `{{{expr}}}` - raw/unescaped output (only for trusted content)

Text bindings do path lookups only. They cannot do arithmetic or call functions.

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

Operators: `==`, `!=`, `>`, `<`, `>=`, `<=`, `&&`, `||`, `!`

**Constraints:** max 5 logical operators per expression; cannot mix `&&` and
`||` in one expression; no parentheses for grouping; no ternary; **no
arithmetic**.

Each side of a comparison is either a literal (number, quoted string, `true`,
`false`) or a dotted state path. Anything else is read as a path, so
`{{currentIndex == items.length - 1}}` looks up a key literally named
`items.length - 1`, finds nothing, and the condition is silently false. Have the
server send a precomputed `lastIndex` (or `isLast`) instead. Bare `.length` on an
array or string does work: `<if condition="items.length > 3">`.

### Loops

```html
<for each="item in items">
  <div>{{item.name}} - {{item.price}}</div>
</for>

<for each="item in reorderableItems">
  <todo-row key="{{item.id}}" title="{{item.title}}"></todo-row>
</for>
```

- The collection must be a JSON array.
- Nested loops are supported; outer loop variables remain accessible.
- Repeats reconcile by array position by default; item attributes never act as
  keys, and `data-key` is an ordinary application attribute.
- Add compiler-only `key="{{item.id}}"` to the first concrete child to preserve
  identity across reorder. Leading `<if>` wrappers are transparent; `key`
  directly on `<if>`, `<for>`, or `<outlet>` is invalid.
- `key="{{item}}"` supports arrays of unique string or finite-number primitives.
  Key paths must be rooted at the loop variable or the build fails with
  `invalid-for-key`. Duplicate or invalid runtime keys warn once and fall back
  to positions.
- **Components inside loops do NOT inherit loop variables.** Pass data via
  attributes:

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
     Compare against existing state instead of creating mirror observables. -->
<button ?disabled="{{currentIndex == 0}}">Prev</button>
<button ?disabled="{{currentIndex == lastIndex}}">Next</button>
<option ?selected="{{item.id == selectedId}}">{{item.name}}</option>

<!-- Mixed static + dynamic -->
<img src="/img/{{user.avatar}}" alt="{{user.name}}" />

<!-- Complex/property binding -->
<my-widget :config="{{settings}}"></my-widget>
```

`?attr` is also the styling hook. Bind a `data-*` attribute and select on it in
CSS rather than touching `classList` from JavaScript.

Property bindings use `:` to write directly to DOM properties. For
client-created trees, initial property bindings are applied before a child's
`connectedCallback` runs, so children can read parent-provided values during
setup and still receive later updates through the live binding.

### Events

```html
<button @click="{handleClick()}">Click me</button>
<input @keydown="{onKeydown(e)}" />
<button @click="{selectItem(item.id, 'details', e)}">Select</button>
<div @mouseenter="{onHover()}" @mouseleave="{onLeave()}">Hover</div>
```

Handler arguments can be `e`, dotted component or repeat-scope paths, or
string/number/boolean/null literals. Nested JavaScript expressions are not
parsed in templates. An `@event` requires a `.ts` file on that component.

Each `@event` gets its own listener on the element it is written on - bindings
are never delegated to a shared root. Non-bubbling events (`@focus`, `@blur`,
`@mouseenter`, `@load`, `@error`, `@toggle`) therefore work, and an ancestor's
bubble-phase `stopPropagation()` cannot suppress them.

### DOM references

```html
<input w-ref="{searchInput}" type="text" />
```

The braces are **required**. A non-braced `w-ref="searchInput"` fails the build
with `invalid-w-ref`. Declare the property in the class:
`searchInput!: HTMLInputElement;`

Use `w-ref` only for imperative browser APIs - `focus()`, `scrollIntoView()`,
`showModal()`, `showPopover()`, measurement. Never use it to read or write
state that a template binding could express.

### The `<template>` tag

Every unwrapped component uses Light DOM:

```html
<!-- my-card.html -->
<h2>{{title}}</h2>
<p>{{description}}</p>
```

Use a sole top-level `<template shadowrootmode="open">` only when the component
needs native `<slot>`, a native Shadow boundary, or root host events on the
component root:

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

The wrapper must contain the complete component. `closed`, a dynamic or invalid
value, placement on another element, multiple declarations, or extra top-level
content fails the build. `<slot>` is a build error in an unwrapped Light
component.

Root host events catch custom events bubbling up from child components. To
cross any Shadow boundary between the child and the listener, an event must
bubble and be `composed`; `this.$emit()` always sets both so Light components
nested in a Shadow tree work too. A hand-built `new CustomEvent(name)` defaults
to neither and will never reach the root - pass
`{ bubbles: true, composed: true }` or bind it on the child element.

The binding sits on the host element, so it also catches events targeted at the
host itself - what host-interactive components (host `tabindex`, presentational
shadow content) rely on. It does not see non-composed events (`change`,
`submit`, `select`, media); bind those per element.

One root listener also serves an arbitrarily large `<for>`, so this is the way
to trade per-row listeners for a single handler on a very long list. For rows
inside the shadow tree `e.target` is the host, so use `e.composedPath()[0]` to
find the element that was hit.

### Outlet

```html
<!-- Parent component template -->
<nav>...</nav>
<main><outlet /></main>
```

`<outlet></outlet>` is also valid, but outlets are empty directives and the
self-closing form is preferred.

### Entry template

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

## Styling

Keep ordinary paired component CSS. The compiler scopes Light CSS, lowers
`:host`, and namespaces static keyframes. Shadow components retain native
Shadow CSS scoping. No CSS-in-JS or styles written from script.

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

.header { font-weight: bold; }
```

- `:host` styles the component root; `:host([attr])` styles by attribute in
  both Light and Shadow components.
- Light DOM preserves normal inheritance while compiler scoping bounds
  component rules. Shadow DOM creates a native boundary.
- Light scoping is applied at build time: every element your template declares
  is stamped with a per-component marker attribute, and every selector is
  qualified against it. **Elements you create imperatively from script (setting
  `innerHTML`, `document.createElement` inside a component) carry no marker and
  are not styled by that component's CSS.** Declare markup in the template, or
  render it through `<if>` / `<for>`, and it is scoped automatically. Creating a
  *component host* from script is fine — its content comes from the compiled
  template. A template that interpolates raw HTML (`{{{expr}}}`) is scoped with a
  native `@scope` enclosure instead, which covers the interpolated markup, as is
  a component whose CSS uses a shape the stamper cannot rewrite (for example
  `:host` nested inside `:is(...)`).
- Shadow-only selectors and unsafe Light keyframe references fail the build.
- `data-wl` and `data-wl-*` are reserved for scoping; authoring either fails the
  build.
- Use CSS custom properties for theming. Nested fallbacks like
  `var(--primary, var(--fallback))` are also discovered as tokens.
- Malformed CSS fails the build, including unterminated `var()` calls,
  comments, strings, and unmatched delimiters.

### Reactive styling

State drives styling through bound attributes, never through script.

```html
<button ?data-active="{{isActive}}" @click="{toggle()}">{{label}}</button>
<article ?data-compact="{{density == 'compact'}}">...</article>
```

```css
button[data-active] { background: var(--accent); color: white; }
button:not([data-active]) { background: transparent; }
article[data-compact] { --row-height: 1.5rem; }
```

This works during SSR, needs no synchronization code, and stays correct when the
server re-renders.

### Modern CSS to prefer

| Feature | Use for |
|---|---|
| `:has()` | Parent/sibling state without a JS class |
| `@container` / `cqi` units | Component-level responsiveness |
| `@starting-style` + `transition-behavior: allow-discrete` | Animating in/out of `display: none`, `popover`, `<dialog>` |
| `color-mix()` / `light-dark()` / `oklch()` | Derived colors, dual themes without JS |
| `@layer` | Predictable cascade ordering |
| `content-visibility: auto` | Long lists without virtualization code |
| `text-wrap: balance` | Headline typography |
| `:user-valid` / `:user-invalid` | Form feedback without validation JS |

### Modern HTML to prefer

| Element / attribute | Replaces |
|---|---|
| `<dialog>` + `showModal()` | Custom modal with overlay and focus trap |
| `popover` / `popovertarget` | JS dropdown, menu, tooltip, toast |
| `<details><summary>` | JS accordion or disclosure |
| `<datalist>` | JS autocomplete |
| `inert` | Manual `tabindex` juggling |
| `loading="lazy"` / `decoding="async"` | IntersectionObserver image loaders |
| `<progress>` / `<meter>` | Div-based bars |
| Native constraint validation | Hand-rolled validators |

### Progressive enhancement only

These are not Baseline across all engines. Use them as an enhancement layer that
degrades cleanly, never as load-bearing layout or behavior.

| Feature | Guard with |
|---|---|
| `anchor-name` / `position-area` | `@supports (anchor-name: --x)`, fall back to static placement |
| Scroll-driven animations | `@supports (animation-timeline: view())` |
| `field-sizing: content` | A sensible fixed `width` / `rows` default |
| `text-wrap: pretty` | Harmless when ignored |
| Customizable `<select>` / `<selectedcontent>` | The native `<select>` rendering |

Never polyfill these in JavaScript. If the fallback is unacceptable, use a
Baseline approach instead.

### Animation

Animation is CSS. There is no supported JavaScript animation path.

```css
/* Enter and exit for a popover or dialog */
.menu-panel {
  transition: transform 200ms ease, overlay 200ms allow-discrete,
              display 200ms allow-discrete;
  transform: translateX(-100%);
}

.menu-panel:popover-open {
  transform: translateX(0);
}

@starting-style {
  .menu-panel:popover-open {
    transform: translateX(-100%);
  }
}

@media (prefers-reduced-motion: reduce) {
  .menu-panel { transition: none; }
}
```

Always honor `prefers-reduced-motion`.

### View transitions

The router wraps every client-side navigation in `document.startViewTransition()`
automatically. **Do not** wrap `Router.navigate()` in your own
`startViewTransition()` - that would double-transition.

Name the transitioning element in **component CSS**:

```css
/* mp-app.css */
.route-surface {
  view-transition-name: page-content;
}
```

Declare the animations in the **entry template's document-level `<style>`**.
`::view-transition-*` pseudo-elements live on the document root and cannot be
reached from inside a shadow root:

```html
<!-- index.html -->
<style>
  ::view-transition-old(page-content) {
    animation: 200ms ease-out both fade-out;
  }
  ::view-transition-new(page-content) {
    animation: 300ms ease-in both fade-in;
  }
  @keyframes fade-in  { from { opacity: 0; } }
  @keyframes fade-out { to   { opacity: 0; } }
</style>
```

While the router is active it installs a nonce-bearing
`@view-transition { navigation: none; }` override, because automatic
cross-document transitions conflict with intercepted routes that fall back to
SSR document requests. `Router.destroy()` removes the override. The router
awaits `updateCallbackDone` (not `.finished`) so rapid navigations supersede
each other without queuing.

## Interactivity

### Start scriptless

Most components should have only `.html` and `.css`. A scriptless component
renders bindings, `<if>`, and `<for>` on the server and contributes no bootstrap
state to the client.

```html
<!-- user-card.html - no .ts file, nothing to add -->
<h2>{{user.name}}</h2>
<p>{{user.title}}</p>
<if condition="user.isAdmin">
  <span class="badge">Admin</span>
</if>
```

### When to add a `.ts` file

Add one only when at least one of these is true:

1. The template has an `@event` handler.
2. The template has a `w-ref` you need for an imperative browser API.
3. You need `connectedCallback` / `disconnectedCallback` lifecycle work.
4. You need to fetch data or call a browser API.
5. The component exposes imperative methods or a public property API.

If none apply, do not create the file.

### When to add `@observable` or `@attr`

Same test, one level down. These decorators exist to connect a value to
JavaScript - not to make it render.

- **`@observable`** - only when TypeScript in this component reads or writes the
  value after hydration. A value that is rendered once and never changed on the
  client belongs in the server state JSON.
- **`@attr`** - only when the value is part of the component's public API:
  another component, a parent template, or the router sets it as an HTML
  attribute.

```typescript
// Justified: the click handler mutates it, the template renders it.
@observable count = 0;
increment(): void { this.count += 1; }

// Justified: a parent template sets label="..." on this element.
@attr label = '';

// NOT justified: nothing in TypeScript touches it - put it in state JSON.
@observable heading = 'Welcome';

// NOT justified: mirrors an expression the template can evaluate.
@observable hasItems = false;   // use <if condition="items.length">
@observable prevDisabled = true; // use ?disabled="{{currentIndex == 0}}"
```

### The component class

```typescript
import { WebUIElement, attr, observable } from '@microsoft/webui-framework';

export class MyComponent extends WebUIElement {
  @attr label = 'Default';
  @attr({ mode: 'boolean' }) disabled = false;

  @observable count = 0;
  @observable items: Item[] = [];

  // Populated by w-ref="{inputEl}" in the template
  inputEl!: HTMLInputElement;

  onSubmit(): void {
    const text = this.inputEl.value.trim();
    if (!text) return;
    this.items = [...this.items, { text, price: 0 }];
    this.inputEl.value = '';
  }

  onKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter') this.onSubmit();
  }

  onItemDelete(e: CustomEvent<{ id: string }>): void {
    this.items = this.items.filter(i => i.id !== e.detail.id);
  }
}

MyComponent.define('my-component');
```

For a component with many initially offscreen SSR instances, put the complete
policy on its root template:

```html
<template w-render="lazy" w-reserve-block-size="72px">
  <!-- Component content -->
</template>
```

This combines visibility-deferred hydration with `content-visibility: auto`.
The reservation is required and should approximate one instance's normal block
size. Use `<template w-hydrate="lazy">` only when hydration should defer but
rendering containment is unsafe.

Import the optional coordinator once before component definitions:

```typescript
import '@microsoft/webui-framework/lazy-hydration.js';
import './feed-item.js';
```

On an instance, `w-hydrate="eager"` keeps rendering deferral but hydrates
immediately; `w-render="eager"` disables both. Use `hydratedCallback()` for work
that requires bindings or refs. Missing coordinator or browser support falls
back to eager hydration. Visibility-deferred hydration does not delay image
fetching; use native `loading="lazy"` and reconcile an already-complete `w-ref`
image from `hydratedCallback()` when component state depends on `@load` or
`@error`.

| Decorator | Purpose | SSR? | Triggers DOM update? |
|---|---|---|---|
| `@attr` | HTML attribute reflection | Yes; an existing SSR host attribute wins | Yes |
| `@attr({ mode: 'boolean' })` | Boolean attribute (present/absent) | Yes; host presence wins | Yes |
| `@observable` | Reactive state used by TypeScript | Yes (from JSON state) | Yes |

| Method / property | Description |
|---|---|
| `this.$emit(name, detail?)` | Dispatch a bubbling CustomEvent |
| `this.$update()` | Force a reactive update cycle |
| `this.$flushUpdates()` | Synchronously flush pending updates |
| `protected hydratedCallback()` | Run synchronously once after the first successful hydration or client mount |
| `static define(tagName)` | Register as a custom element |
| `defineComponentAssets(manifest)` | Lazy component asset graphs with `preload(tag)` / `create(tag)` |

### Custom events

```typescript
// Child
this.$emit('item-selected', { id: this.id, name: this.name });
```

```html
<!-- Parent template -->
<child-component @item-selected="{onItemSelected(e)}"></child-component>
```

```typescript
// Parent
onItemSelected(e: CustomEvent): void {
  this.selectedId = e.detail.id;
}
```

### Hydration entry point

```typescript
// index.ts
import './app-shell/app-shell.js';
import './user-card/user-card.js';
```

Importing a component module registers it as a custom element, which triggers
hydration. Nothing else is required.

### The hydration boundary

Never write `@observable` values before hydration. During SSR hydration the
server-rendered DOM is trusted and not re-rendered, so a value set in a field
initializer, the constructor, or before `super.connectedCallback()` cannot
reach the DOM. The write is dropped and the runtime logs a `[WebUI] Hydration
mismatch` warning (development-only; stripped from production via
`__WEBUI_DEV__`).

If the value must appear in the first render, put it in the SSR state JSON.
Otherwise assign it in `hydratedCallback()`. On buffered SSR and client-created
mounts, `super.connectedCallback()` hydrates synchronously, but streamed hosts
and visibility-policy hosts (without an eager instance override) can return
while still deferred.
`hydratedCallback()` is the cross-mode signal: it runs synchronously exactly
once after the first successful hydration or mount, and reconnects or callback
exceptions do not retry it. Once a host has deferred, later state writes
are retained and replayed; this exception does not make constructor or
pre-`super.connectedCallback()` writes safe.

Load buffered definitions through a parser-inserted, non-async ES module script
or a classic `defer` script. Descendants must not structurally mutate a
containing WebUI component's SSR subtree before it hydrates - insertion,
removal, or reordering shifts compiled element indices.

### Progressive streaming hydration

Use `<boundary>` only when the Rust server calls
`WebUIHandler::render_streaming` / `WebUIHandler::stream_response` with a
`FlushWriter`, or when an API backend returns the versioned
`application/x-webui-stream` control format to `webui serve --api-port`. The
directive is removed at compile time and emits no application DOM wrapper.

```html
<head>
  <script type="module" async src="/index.js"></script>
</head>
<body>
  <boundary name="weather-shell">
    <weather-panel status="loading"></weather-panel>
  </boundary>

  <boundary name="critical-composer">
    <message-composer></message-composer>
  </boundary>
</body>
```

- `name` is required, non-empty, static, and unique in the entry template. It
  cannot contain a <code v-pre>{{binding}}</code>.
- Author boundaries only in the outermost entry template. They cannot appear
  inside reusable components, route-shell components, `<if>`, `<for>`,
  `<route>`, or another boundary. An entry-level boundary can fully wrap those
  complete scopes.
- Every registered WebUI component rendered in streaming mode must be inside
  an explicit boundary. Native HTML and unregistered static tail markup may
  remain outside.
- Never author `<webui-hydrate>`. It is reserved generated runtime output.
- Put the async application module in `<head>` before boundary content and
  import `@microsoft/webui-framework/streaming.js` before component
  registration modules.
- Boundary HTML commits strictly in document order. For slow backend state,
  commit a complete component shell as `BoundaryMode::Updatable`, then call
  `StreamingResponse::update` when data resolves. Updates interleave on the
  original response and call `setState()` without rerunning hydration or
  `hydratedCallback()`.
- Resolve free-form names once with `StreamingResponse::boundary`; hot writes
  use integer `BoundaryId` handles. Call `write_shell`, ordered
  `write_boundary`, interleavable `update`, then `finish`.
- `webui:boundary-hydrated` is emitted only when
  `window.__WEBUI_STREAMING_DEBUG__ === true`; its `detail.kind` is
  `checkpoint`, `update`, or `terminal`. Every commit also emits an
  unconditional `performance.mark()` (`webui:boundary:<id>`,
  `webui:boundary:<id>:update`, `webui:streaming:terminal`) that tooling can
  read retroactively without a listener.
  `webui:hydration-complete` fires only after the terminal record and eager
  pending hydration work complete. Visibility-deferred lazy roots do not keep
  this one-shot startup event open.
- `window.__WEBUI_STREAMING_SLICE_MS__` opts into a time-sliced drain that
  yields between boundaries. Use it only when an intermediary coalesces the
  response into one chunk; it costs total hydration time.
- A semantic flush hands bytes to the HTTP transport. Server adapters,
  compression, proxies, and CDNs can still buffer them.

Malformed directives use stable diagnostics:
`missing-boundary-name`, `invalid-boundary-name`,
`duplicate-boundary-name`, `nested-boundary`, `boundary-crosses-scope`, and
`authored-webui-hydrate`.

Dynamic append streams, out-of-order replacement, router-stream reuse, direct
Node/FFI/WASM response sessions, and declarative partial updates are not part of
this contract. Node can drive the CLI bridge, but the CLI remains the Rust
session and transport owner.

### Lazy component mounting

This is the **only** sanctioned place to insert an element from JavaScript,
because a lazily loaded component has no template representation until fetched.

Prefer the template-driven form, which needs no DOM insertion at all:

```typescript
await Router.ensureLoaded('settings-dialog');
this.showSettings = true;
```

```html
<if condition="showSettings">
  <settings-dialog @close="{onCloseSettings()}"></settings-dialog>
</if>
```

Without `@microsoft/webui-router`, prebuild the asset and mount it into a
`w-ref` slot:

```bash
webui build ./src --out ./dist --plugin=webui \
  --emit-component-assets settings-dialog \
  --metafile ./dist/component-assets-meta.json
```

```typescript
import { defineComponentAssets } from '@microsoft/webui-framework/component-asset.js';

export const settingsAssets = defineComponentAssets({
  'settings-dialog': {
    asset: '/settings-dialog.webui.js',
    module: () => import('./settings-dialog/settings-dialog.js'),
  },
});

async onOpenSettings(): Promise<void> {
  settingsAssets.preload('settings-dialog');
  this.panelSlot.replaceChildren(await settingsAssets.create('settings-dialog'));
}
```

Assets keep entry-owned templates external, inline dependencies used by one
asset root, and split dependencies shared by multiple roots into deduplicated
dynamic chunks. Do not copy generated chunk filenames into the manifest; each
root asset carries its own dynamic imports. `create(tag)` waits for the template
graph and module, then creates the element. The normal entry bundle must load
first. Component assets cannot be combined with `<route>`; use the router for
routed components.

## Routing

```html
<route path="/" component="app-shell">
  <route path="" component="home-page" exact />
  <route path="users" component="user-list" exact />
  <route path="users/:id" component="user-detail" exact />
</route>
```

- Child paths are relative to the parent (no leading `/`).
- Use `exact` on leaf routes; omit it on parents that have `<outlet />`.
- Path params: `:id` (required), `:query?` (optional), `*path` (catch-all).
- Initial SSR delivers CSS only for the matched route chain; inactive route
  styles remain deferred until navigation.

| Attribute | Example | Description |
|---|---|---|
| `path` | `"users/:id"` | URL path template (relative to parent) |
| `component` | `"user-detail"` | Component tag to mount |
| `exact` | (boolean) | Require exact path match |
| `query` | `"action,to,subject"` | Allowlist of query params set as attributes (deny-by-default) |
| `keep-alive` | (boolean) | Preserve DOM and local state across navigations |
| `cache-tags` | `"thread:{threadId},inbox"` | Cache tag templates resolved at render time |
| `invalidates` | `"inbox,sent,counts"` | Tags auto-invalidated after mutation actions |
| `pending` | `"loading-skeleton"` | Loading UI for slow navigations (>150ms) |
| `error` | `"error-display"` | Error boundary on fetch failure |

All attributes are validated at build time. Referencing a non-existent `pending`
or `error` component is a compile error.

```typescript
Router.start({
  loaders: {
    'home-page': () => import('./pages/home-page.js'),
    'user-detail': () => import('./pages/user-detail.js'),
  },
});
```

Route loaders (`static loader({ params, query, signal })`) and actions
(`static action({ formData, params, signal })`, enabled by
`Router.start({ actions: true })`) live on the component class. Cache and
preload are optional runtime tiers; a default `Router.start()` does not load the
cache module.

Every route intended for partial navigation must register a custom element.
Scriptless templates are registered by the compiler-owned host runtime. If a
route remains unregistered after template publication and loader resolution, the
router navigates the document so the server can render it.

Full detail: [Routing](/guide/concepts/routing).

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

**Missing paths:** text bindings render empty, `<if>` evaluates false. No error.

**Route-scoped state.** Each route handler should return only the keys that
route's template binds to. Sending full app state on every route wastes
bandwidth and render time.

### Reserved `$webui` state

The top-level `"$webui"` object is reserved for trusted host HTML emitted at
document boundaries:

```json
{
  "$webui": {
    "headEnd": "<link rel=\"preload\" as=\"image\" href=\"/hero.avif\">",
    "bodyStart": "<!-- immediately after <body> -->",
    "bodyEnd": "<script src=\"/livereload.js\"></script>"
  }
}
```

All three members are optional strings. `headEnd`, `bodyStart`, and `bodyEnd`
are emitted raw immediately before `</head>`, after `<body>`, and before
`</body>`, respectively. Missing, empty, `null`, or non-string members are
ignored. WebUI strips the reserved object from hydration and partial-navigation
state, so client-side templates and code cannot read it.

**Never put request-derived or otherwise untrusted content in `$webui`.** The
values are not escaped and can create an XSS vulnerability. See
[Integrations](/guide/integrations/) for host-specific rendering details.

### Truthiness

| Value | Truthy? |
|---|---|
| `true` | Yes |
| `false` | No |
| `0` | No |
| Non-zero number | Yes |
| `""` (empty string) | No |
| `"false"` (string!) | **Yes** (non-empty string) |
| `[]` (empty array) | No (server) / **Yes** (client) - always use `.length` |
| `{}` (empty object) | No (server) / **Yes** (client) - test a real field |
| `null` / missing key | No |

**Never use the string `"false"` for boolean state. Use real booleans.**

**Never test a bare array or object for truthiness.** The server evaluator treats
empty collections as falsy; the compiled client condition is plain JS `!!value`,
where `[]` and `{}` are truthy. Writing `<if condition="items">` means SSR and
hydration can disagree. Write `<if condition="items.length">` instead - that
agrees on both sides.

## Anti-patterns

### Building UI from JavaScript

```typescript
// WRONG
const row = document.createElement('div');
row.textContent = item.name;
this.list.appendChild(row);

// WRONG
this.list.innerHTML = items.map(i => `<div>${i.name}</div>`).join('');
```

```html
<!-- RIGHT -->
<for each="item in items">
  <div>{{item.name}}</div>
</for>
```

### Styling from JavaScript

```typescript
// WRONG
this.panel.style.background = this.isActive ? 'blue' : 'transparent';
this.panel.classList.toggle('active', this.isActive);
this.shadowRoot.adoptedStyleSheets = [sheet];
```

```html
<!-- RIGHT -->
<div class="panel" ?data-active="{{isActive}}">...</div>
```

```css
.panel[data-active] { background: var(--accent); }
```

### Animating from JavaScript

```typescript
// WRONG
el.animate([{ opacity: 0 }, { opacity: 1 }], { duration: 200 });
// WRONG
let o = 0; setInterval(() => { el.style.opacity = String(o += 0.05); }, 16);
```

```css
/* RIGHT */
.toast { transition: opacity 200ms ease; opacity: 1; }
@starting-style { .toast { opacity: 0; } }
```

### Unnecessary framework surface

```typescript
// WRONG - nothing in TypeScript touches these
@observable heading = 'Welcome';
@observable subtitle = 'Get started below';
```

```json
// RIGHT - server state JSON
{ "heading": "Welcome", "subtitle": "Get started below" }
```

### Shadow observables that mirror an expression

```typescript
// WRONG
@observable items: Item[] = [];
@observable hasItems = false;
onItemsChanged(): void { this.hasItems = this.items.length > 0; }
```

```html
<!-- RIGHT -->
<if condition="items.length">...</if>
<button ?disabled="{{currentIndex == 0}}">Prev</button>
<option ?selected="{{app.slug == currentApp.slug}}">{{app.name}}</option>
```

Loop variables compose with outer component state in the same expression, so
per-item flags like `isCurrent` in the SSR JSON are almost never needed.

Text bindings do path lookups only. If you need `{{currentIndex + 1}}` for a
1-based display, that is a legitimate `@observable` or a precomputed state key.

### Reading DOM instead of state

```typescript
// WRONG
const value = this.shadowRoot.querySelector('.count').textContent;
```

Use `@observable` for state TypeScript changes, template bindings for output,
and `w-ref` only for imperative APIs.

### Hand-built platform primitives

```html
<!-- WRONG -->
<div class="modal-backdrop" @click="{close()}">
  <div class="modal" role="dialog">...</div>
</div>

<!-- RIGHT -->
<dialog w-ref="{dialogEl}" @close="{onClose()}">...</dialog>
```

`showModal()` gives focus trapping, `inert` backdrop, Escape handling, and
top-layer stacking for free.

### Also not supported

1. **No ternary in templates.** `{{x ? 'yes' : 'no'}}` does not work.
2. **No function calls in bindings.** `{{formatDate(item.date)}}` does not work.
   Compute on the server or in an event handler.
3. **No mixed `&&` and `||`** in one condition. Split into nested `<if>` blocks.
4. **No parentheses in conditions.**
5. **No JavaScript in HTML templates.** Templates compile to binary.
6. **No JavaScript in CSS.** Use custom properties for dynamic values.
7. **No computed getters for SSR state.**
8. **Components inside `<for>` do NOT inherit loop variables.**
9. **No `import` or `require` in templates.** Components are discovered by file
   naming convention.
10. **Non-braced `w-ref`** fails the build. Use `w-ref="{name}"`.

## Pre-flight checklist

Before emitting WebUI code, confirm:

- [ ] No `createElement`, `innerHTML`, `appendChild`, or `insertAdjacentHTML`,
      except mounting a lazily loaded component.
- [ ] No `.style.x =`, `classList`, `setAttribute('style')`, or
      `adoptedStyleSheets`.
- [ ] No `element.animate()`, `requestAnimationFrame` tweening, or animation
      library. Animation is in `.css`.
- [ ] Every `.ts` file exists because of an event, `w-ref`, lifecycle hook,
      fetch, or public API - not by default.
- [ ] Every `@observable` / `@attr` is read or written by TypeScript, or is
      public API. Otherwise it moved to the state JSON.
- [ ] No observable mirrors an expression the template can evaluate.
- [ ] Every `{{binding}}`, `<if>`, and `<for>` path exists in the state JSON.
- [ ] Every `w-ref` uses braces.
- [ ] A built-in element was considered before a hand-built one
      (`<dialog>`, `popover`, `<details>`).
- [ ] `::view-transition-*` rules are in the entry template, not component CSS.
- [ ] `prefers-reduced-motion` is honored wherever motion is used.
- [ ] No conditions mix `&&` with `||`, use parentheses, or use a ternary.
- [ ] Every native `<slot>` is inside a component whose sole top-level element
      is `<template shadowrootmode="open">`.

## Build and run

```bash
# Dev server with live reload
webui serve ./src --state ./data/state.json --plugin=webui --watch

# Production build
webui build ./src --out ./dist --plugin=webui

# Inspect the compiled protocol
webui inspect ./dist/protocol.bin
```

Common flags on both commands: `--entry`, `--css <link|style|module>`,
`--css-bundle` (merge component stylesheets into shared chunks; not valid with
`--css module`), `--components`, `--theme`,
`--projection-manifest`, `--emit-component-assets`, `--metafile`,
`--format json`.

Every unwrapped component is Light. A component uses Shadow only when its
complete template is a sole top-level `<template shadowrootmode="open">`.

Authoring mistakes fail the build with a structured diagnostic carrying a stable
code, source location, snippet, and a `help:` fix. Branch on the `code`, never
the message. `--format json` emits one JSON object per error on stdout.

```
x error: invalid <for> each expression [invalid-for-each]
  --> index.html:67:5
    each="person inpeople"
  help: use the form each="item in collection", e.g. each="todo in todos"
```

Full flag tables, exit codes, and the error-code list:
[CLI Reference](/guide/cli/).

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

Add `@microsoft/webui-router` for client-side navigation. Passing
`--projection-manifest` narrows hydration state to exactly the `@observable` and
`@attr` fields your bundle uses; omitting it keeps full state.

## Server integration

Any backend loads `protocol.bin` once and renders with JSON state per request.

```rust
let protocol = Protocol::from_protobuf(&fs::read("dist/protocol.bin")?)?;
let handler = WebUIHandler::new();
handler.render(&protocol, &state, &options, &mut writer)?;
```

```javascript
const protocol = new Protocol(result.protocol, { plugin: 'webui' });
const html = protocol.render(state, { entry: 'index.html', requestPath: req.url });
```

For Rust progressive hydration, create a `StreamingResponse` over a
`FlushWriter`. Keep one bounded worker as the response owner, cap admitted
renders before spawning it, and configure the transport's flush timeout.

```rust
let mut page = handler.stream_response(&protocol, &options, &mut writer)?;
let critical = page.boundary("critical-composer")?;
page.write_shell(&state)?;
page.write_boundary(critical, &state, BoundaryMode::Final)?;
page.finish(&state)?;
```

With `webui serve --api-port`, a Node or other HTTP backend can return
newline-delimited control records instead:

```text
{"type":"shell","version":1,"state":{...}}
{"type":"boundary","name":"critical-composer"}
{"type":"finish"}
```

Honor HTTP write backpressure and cap concurrent streams. The CLI uses a
capacity-one command channel, resolves boundary names once, and keeps the
compiled protocol plus browser-facing bytes in Rust. Returning JSON keeps the
buffered state path.

If the backend refuses a stream request (non-success status such as a `503`
from its own concurrency cap), no boundary was ever sent, so `webui serve` logs
one warning and renders the page from fallback state rather than replacing the
app with the upstream error body. A failure *after* the stream is live still
fails the response, because boundaries already flushed cannot be rewound.

Equivalent APIs exist for WebAssembly, Python (FFI), Go (cgo), and C#. For
`Router.ensureLoaded()`, expose `GET /_webui/templates?t=tag1,tag2` backed by
`render_component_templates(&tags, &inv)`.

Full detail: [Integrations](/guide/integrations/).

## Where to read more

| Topic | Page |
|---|---|
| Directives (`if`, `for`, attributes, `route`) | [/guide/concepts/directives/](/guide/concepts/directives/) |
| Component authoring and hydration | [/guide/concepts/interactivity](/guide/concepts/interactivity) |
| Best practices and React-habit pitfalls | [/guide/concepts/best-practices](/guide/concepts/best-practices) |
| Routing, loaders, actions, caching | [/guide/concepts/routing](/guide/concepts/routing) |
| Design tokens and theming | [/guide/concepts/css-tokens](/guide/concepts/css-tokens) |
| CLI flags, diagnostics, exit codes | [/guide/cli/](/guide/cli/) |
| Rust, Node, WASM, FFI, Electron | [/guide/integrations/](/guide/integrations/) |
| Performance characteristics | [/guide/concepts/performance](/guide/concepts/performance) |
| Build a first app | [/tutorials/hello-world/](/tutorials/hello-world/) |
