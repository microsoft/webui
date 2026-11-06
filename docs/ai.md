---
layout: page
title: "WebUI authoring reference"
description: "Core application-authoring rules for coding agents: composition, templates, CSS, state, browser behavior, and task-specific guides."
---

# WebUI Framework - AI Reference

> **Authoring reference for coding agents.** Read the rules before generating
> WebUI application code. Use the linked guides for detailed API options and
> platform-specific constraints.
>
> Install the loader skill in your project with
> `npx skills add microsoft/webui --skill webui-reference`, or add `--global`
> to use it across projects. See
> [AI Coding Agents](/start/installation#ai-coding-agents).
> In a consuming app it reads the installed `@microsoft/webui/ai.md` and the
> relevant guides bundled alongside it. Upgrading `@microsoft/webui` updates
> both. In the framework source checkout, it reads `docs/ai.md` and the
> corresponding source guides.
>
> In source Markdown, links starting with `/start/`, `/learn/`, `/build/`, or
> `/reference/` are relative to the `docs/` root. The installed package uses
> relative links to its local Markdown files. The
> [documentation site](https://microsoft.github.io/webui/) tracks the current
> release and may differ from an older installed version.

## How to use this reference

Use this page for everyday UI authoring and composition. Specialized features
are opt-in, not prerequisites for a WebUI application. Before implementing one,
read its guide rather than inferring setup or API details from this summary.
In the framework source checkout, use the corresponding file under `docs/`.
In a consuming app, follow the local links from this file to guides in the same
installed package. Keep `@microsoft/webui` and `@microsoft/webui-framework` at
matching versions; do not substitute the latest website for a missing guide.

| When the task involves | Read before implementing |
|---|---|
| On-demand UI and lazy conditional bodies | [Lazy UI](/learn/lazy) |
| Deferred rendering or hydration | [Policy selection](/learn/directives/lazy) and [hydration setup](/build/hydration) |
| Progressive streaming | [Boundary rules](/learn/directives/boundary) and [streaming integration](/build/hydration#progressive-streaming-hydration) |
| Routing or separately loaded component assets | [Routing](/build/routing) or [static component assets](/learn/interactivity#loading-static-component-assets) |
| Desktop apps, native capabilities, or IPC | [Desktop guide](/build/desktop) |
| Markdown sites built with WebUI Press | [Press reference](/reference/webui-press) |
| Server bindings or custom build pipelines | [Integrations](/build/integrations/) and the [CLI reference](/reference/cli/) |

## Rules

Keep structure in templates, styling in CSS, and authored JavaScript focused
on browser behavior.

### 1. The template is the UI

All UI structure lives in `.html` template files. There is no other way to
create UI.

- Never `document.createElement()`, `innerHTML`, `insertAdjacentHTML`,
  `appendChild`, `cloneNode`, or `new DOMParser()` to build UI.
- Show and hide with `<if>`. Repeat with `<for>`. Swap regions with `<outlet>`.
- If the markup you need does not exist yet, add it to the template and gate it
  with `<if>` - do not build it from JavaScript.
- **The single exception** is mounting a lazily loaded component through the
  component-asset API into an explicit `w-ref` slot. Prefer template-driven
  mounting where available. See
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

**Trusted Types:** WebUI automatically uses its private `webui` policy for compiled
templates and generated CSS import maps. No setup call or special import order is
needed. To enforce Trusted Types, use `require-trusted-types-for 'script'; trusted-types webui`
alongside the normal nonce CSP. Share one framework module across bundles; do not
pre-create its policy. Raw triple-brace state HTML, FAST strings and arbitrary
scripts/URLs are not promoted. Client-side router partial navigation is not yet
supported under enforcement. See
[Trusted Types](/build/hydration#trusted-types).

```text
HTML + CSS                  -- build -->          protocol.bin
protocol.bin + JSON state    -- native render -->  HTML
HTML + optional browser JS   -- hydrate -->        interactive components
```

Build templates with WebUI and bundle authored browser code separately.
A supported native host loads the protocol once and supplies JSON state for
each render. Ordinary SSR sends HTML and assets to the browser, not
`protocol.bin`. Browser interactivity uses the optional framework runtime;
static HTML still has transfer, parsing, layout, paint, and memory costs.

1. **Every template binding should exist in the server state JSON.** If the
   template uses `{{title}}`, the server must provide `{ "title": "..." }`.
   Missing text paths render empty. Whole-binding attributes are omitted for
   missing or `null` values; `""`, `0`, and `false` remain attribute values.
   Mixed attributes retain their static text. A missing condition identifier is
   falsy, so `path` is false and `!path` is true. No error is raised.
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
|  |  |- my-component.css     <- CSS (global in Light DOM, isolated in Shadow)
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

### Lazy conditional bodies

```html
<if condition="settingsOpen" lazy>
  <h2>Settings</h2>
  <p>{{displayName}}</p>
</if>
```

```typescript
// Setting the condition is the whole API: the framework loads and renders the body.
this.settingsOpen = true;

// Only when code needs the body's DOM right away, such as before showModal():
await this.$whenRendered();
this.settingsDialog.showModal();
```

- The body is an ordinary template fragment, not a component. It keeps owner
  state, event methods, refs, and loop scope. Shadow/Light behavior is unchanged.
- An initially false body is omitted until needed; an initially true body
  renders on the server and hydrates normally.
- Before `startRuntime()`, import the generated `lazy.webui.js` for the
  integration. Generation writes it and `lazy.webui.d.ts` to `--modules-out`
  (default `--out`).
  Never edit generated files or register custom loaders. Without lazy regions,
  skip the integration.
- Bare `lazy` is demand-loaded. Use `$prefetch(path?)` for selective warming;
  `lazy="intent"` warms on interactions anywhere in the owner. Read the guide
  before choosing owner-wide intent, idle, or eager prefetch.
- Inline bodies do not split their owner's JavaScript. Put expensive behavior
  in independently loadable child components and enable bundler code splitting.
  The integration loads templates and component scripts before rendering.
- State remains application-owned; lazy regions never fetch it. Await
  `$whenRendered()` only before using body DOM. Handle activation failures through
  its rejection or the owner's `lazyerror` event; do not silently ignore them.
- Where `AbortSignal.timeout` is supported, pack fetches time out after
  10 seconds. That deadline does not bound component script imports or
  stylesheet readiness; stylesheet waits have no deadline.
- Closing a region does not unload JavaScript. Let the framework manage template
  lifetimes rather than caching packs yourself.
- `<if>` accepts only `condition` and `lazy`. `id` and other attributes fail
  the build. Rebuild protocol, packs, and client together.
- Small or immediately used UI is cheaper as an ordinary `<if>`.
- Do not combine this protocol-backed feature with `--emit-component-assets`.

Before adding lazy UI, follow the [lazy UI guide](/learn/lazy) for build/runtime
setup, module mapping, prefetch behavior, and error handling.

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
- Recursive trees use a file-local `id` on a defining loop and self-closing
  references with the same item variable. Write `each="item in items"` without
  binding braces:

  ```html
  <for id="tree-item" each="item in items">
    <div>{{item.name}}</div>
    <for id="tree-item" each="item in item.children" />
  </for>
  ```

  Define each ID once per file. The definition renders normally, references
  can precede it, and other files may reuse the ID independently. Missing or
  empty child arrays end the recursion; pass finite trees, not cyclic data.
  The recursive `item in item.children` resolves the parent's children first,
  then shadows `item` for the shared body and restores it on return. A second
  body for the same ID is a `duplicate-for-id` build error.
  Use `id`, not `template`; the removed `template` attribute is a build error.
  Native WebUI supports recursive client updates; FAST components reject named
  references with an actionable build error.
  [Recursive loops](/learn/directives/for#recursive-loops) covers
  validation and keys.
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

Read that property, not a `[w-ref]` DOM attribute. The authoring directive is
removed from delivered HTML.

Use `w-ref` only for imperative browser APIs - `focus()`, `scrollIntoView()`,
`showModal()`, `showPopover()`, measurement. Never use it to read or write
state that a template binding could express.

Refs inside conditional or repeated content are cleared to `undefined` when
their owning view is removed. Declare those refs as optional properties.

`w-ref` is scalar. Reusing one ref name inside `<for>` overwrites the same
property, so the last wired occurrence wins; repeat `key` and position do not
create an indexed ref collection. For item lookup, use a stable authored `id`
with `Document.getElementById()` / `ShadowRoot.getElementById()`, or put the ref
inside an item component.

### The `<template>` tag

Unwrapped components default to Shadow. In a `--dom light` build they use
authored/global Light DOM:

A sole bare top-level `<template>` is also an explicit Light wrapper and is
unwrapped, even when the build fallback is Shadow. Templates with attributes or
`w-render`/`w-hydrate` are not mode selectors; nested templates remain inert
template content.

```html
<!-- my-card.html -->
<h2>{{title}}</h2>
<p>{{description}}</p>
```

In a Light build, use a sole top-level
`<template shadowrootmode="open">` when the component must remain Shadow for a
native `<slot>`, native isolation, CSS-heavy frequent restyling, or root host
events:

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
content fails the build. `<slot>` is a build error only when the effective mode
is Light.

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
self-closing form is preferred. Use one outlet at each route level, including
outlets inside nested layout components and directives; extra outlets produce a
`multiple-outlets` build warning. Child routes have their own outlet level.

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

Keep ordinary paired component CSS. Light CSS is authored/global in its owning
Document or ShadowRoot; selectors, keyframes, and cascade layers are not
rewritten. Shadow components retain native Shadow CSS scoping. No CSS-in-JS or
styles written from script.

```css
/* my-component.css */
my-component {
  display: block;
  padding: 1rem;
}

my-component[disabled] {
  opacity: 0.5;
  pointer-events: none;
}

my-component[variant="primary"] {
  background: var(--colorBrandBackground);
}

.header { font-weight: bold; }
```

- `:host` styles the component root only in Shadow DOM. In Light DOM, target
  the component tag directly, for example `my-card` or `my-card[disabled]`.
- Light DOM preserves normal inheritance and global cascade. Parent rules can
  reach Light descendants and matching selectors can affect other Light
  components in the same CSS tree.
- `:host`, `:host-context`, and `::slotted` fail with
  `unsupported-light-css` in effective Light components; use ordinary
  selectors or author an open Shadow root.
- `data-wl` and `data-wl-*` are ordinary author attributes; WebUI no longer
  generates or reserves them for Light CSS.
- Use CSS custom properties for theming. Nested fallbacks like
  `var(--primary, var(--fallback))` are also discovered as tokens.
- Scoped token overrides do not replace theme defaults for other elements.
  Use bare `:host` rules for shared Shadow component defaults or `:root` for
  document defaults. See [design tokens](/learn/css-tokens).
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

Select `transitions: viewTransitions` from the delivery `router.js` entry to wrap
route commits in `document.startViewTransition()`. **Do not** wrap `router.navigate()` in your own
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
SSR document requests. `router.destroy()` removes the override. The router
awaits `updateCallbackDone` (not `.finished`) so rapid navigations supersede
each other without queuing. Resizing or superseding the animation does not fail
a committed route; route commit errors still follow normal navigation error handling
without duplicate unhandled transition rejections.

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

### Choose the render and hydration policy

Rendering and hydration are separate decisions:

- **Rendering** is browser layout/paint work for the server-rendered DOM.
- **Hydration** is loading JavaScript and attaching bindings/listeners.

Agents must use this table instead of treating every form of "lazy" as the same
behavior:

| Root template policy | Rendering before activation | Hydration trigger | Use for |
|---|---|---|---|
| No directive | Normal | Eager, when the definition loads | Visible interactive roots and first-use-critical UI |
| `w-hydrate="lazy"` | Normal | Viewport relevance | Offscreen UI where `content-visibility` containment is unsafe |
| `w-render="lazy"` + required `w-reserve-block-size` | `content-visibility: auto` skips offscreen layout/paint | Viewport relevance | Repeated or numerous offscreen components; preferred lazy-row policy |
| `w-hydrate="interaction"` | Normal | Pointer, focus, keyboard, or click intent | One visible app shell/island when startup JS/heap matters more than first-use latency |
| `w-render="lazy"` + reservation + `w-hydrate="interaction"` | `content-visibility: auto` skips offscreen layout/paint | Interaction intent | One offscreen singleton that should defer both rendering work and JavaScript |

Default to eager hydration for visible interactive roots. Use interaction
hydration only for a singleton shell/island, never repeated items; it trades
first-use latency for lower startup JavaScript and heap. Never combine
`w-render="lazy"` with `w-hydrate="lazy"`, and always give `w-render="lazy"` a
realistic `w-reserve-block-size`.

These policies require optional coordinators; attributes alone do not install
them. Before using them, follow the [policy guide](/learn/directives/lazy) and
[hydration setup](/build/hydration), including interaction-loading limitations.
Deferred hydration does not defer image fetching; use native `loading="lazy"`.

### Component API quick reference

| Decorator | Purpose | SSR? | Triggers DOM update? |
|---|---|---|---|
| `@attr` | HTML attribute reflection | Yes; an existing SSR host attribute wins | Yes |
| `@attr({ mode: 'boolean' })` | Boolean attribute (present/absent) | Yes; host presence wins | Yes |
| `@observable` | Reactive state used by TypeScript | Yes (from JSON state) | Yes |

| Method / property | Description |
|---|---|
| `this.$emit(name, detail?)` | Dispatch a bubbling CustomEvent |
| `await this.$whenRendered()` | Wait for pending rendering, activated lazy regions, and affected WebUI descendants before using the updated DOM |
| `<property>Changed(oldValue, newValue)` | For `@attr` and `@observable`, run once after the component mounts with `oldValue` undefined, then synchronously on each live assignment |
| `protected hydratedCallback()` | Run synchronously once after the first successful hydration or client mount |
| `static define(tagName)` | Register as a custom element |
| Generated component asset root | Import a stable generated `<tag>.webui.js` payload, then wrap its default export with `defineComponentAsset()` |

**Change state, await rendering, then use the DOM.** Reactive assignments
request updates automatically:

```typescript
this.showAction = true;
await this.$whenRendered();
this.actionButton.focus(); // w-ref="{actionButton}" on the conditional button
```

Call it on a connected, mounted component. It includes affected WebUI
descendants, not unrelated future work or paint. Handle rejections at your
application's error boundary and recheck focus intent after awaiting. See
[affected rendering](/learn/interactivity#wait-for-affected-rendering)
for the complete contract. Do not count microtasks or use timers to guess when
rendering has finished.

**Render API migration:** `$update(path?)`, `$flushUpdates()`, and lazy
`$rendered()` were intentionally removed. Use reactive assignments to request
rendering; replace observable objects or arrays instead of mutating them in
place. Await `$whenRendered()` before dependent DOM work; it is not a
synchronous flush or an update request.
See [migration guidance](/learn/interactivity#migrating-render-calls).

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

### Composing UI: ownership and boundaries

Compose UI as a tree of templates and components, not as JavaScript that
orchestrates DOM objects. State describes what should be present; templates
describe its structure; WebUI reconciles the DOM.

- **Give each piece of state one owner.** Keep state local when only one
  component needs it; lift shared state to the nearest common owning component.
  Do not keep competing parent and child copies of the same state or synchronize
  them through DOM reads.
- **Pass data down, emit intent up.** Parents supply child inputs through
  bindings. Children emit semantic events with `$emit()`; the owner handles
  application actions and updates its state. Do not mutate parent-owned inputs,
  reach into a parent's implementation, or query siblings to coordinate them.
- **Encapsulate presentation and local behavior.** A child owns its markup,
  CSS, and interactions internal to that responsibility. The owning
  page/controller coordinates application workflows. Choose component boundaries
  around cohesive responsibilities, not merely to move markup into more files;
  ordinary template fragments do not need their own component.
- **Let the template owner control presence.** Express conditional, repeated,
  and routed structure with `<if>`, `<for>`, and `<outlet>`. A child whose
  presence is parent-controlled requests dismissal through an event; it does not
  independently own that visibility or call `this.remove()`.
- **Keep UI instances within their owner's lifetime.** Do not use module-global
  DOM singletons or query, create, and append elements for ordinary UI.
  Use `w-ref` for imperative browser APIs, not to bypass bindings or component
  boundaries.

Loading and composition are separate decisions: an ordinary `<if>` does not
code-split JavaScript. For on-demand UI, load the required definition and assets
before updating visibility through the documented `router.ensureLoaded()` flow,
or use the [lazy conditional integration](#lazy-conditional-bodies), which loads
before rendering. Prefer template-driven mounting; the narrow
[component-asset mounting exception](#lazy-component-mounting) remains supported,
not a general alternative to declarative composition.

### Hydration entry point

```typescript
// index.ts
import { startRuntime } from '@microsoft/webui-framework/runtime.js';
import './app-shell/app-shell.js';
import './user-card/user-card.js';

startRuntime();
```

Component modules declare classes with `.define()`. `startRuntime()` registers
available declarations synchronously; missing metadata waits for delivery.
Later imports register immediately when their metadata is available. Runtime
setup belongs in the application entry, not in every component module.

With a nonce-based CSP, give authored module scripts and host-authored
module-preload links the same response nonce passed to the renderer.
See [CSP and delivery](/build/hydration#csp-and-delivery).

The package root and `runtime.js` are pure API entries. Call `startRuntime()`
after importing selected installers and component modules, including in an
interaction-loaded component-definitions entry. `startRuntime()` automatically
installs Link, Style, and Module component styling; a separate `styles.js`
import is unnecessary. Initial hydration, lazy packs, component assets, and
router responses share the framework's style engine and document-owned catalog.
Streaming and visibility coordinators remain explicit opt-ins.

Optional template collection requires
`@microsoft/webui-framework/template-cache.js` before `startRuntime()` or template lookups and a
matching rebuilt protocol. `collectTemplates(keep?)` releases cache ownership,
not live component dependencies or browser module definitions. See
[runtime entrypoints](/learn/interactivity#runtime-entrypoints).

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

`@attr` reflects immediately once connected and hydrated.
`nameChanged(oldValue, newValue)` runs after this element's refs are wired:
once with `oldValue` undefined for initial state, then synchronously for each
live assignment. Template bindings remain batched in a microtask.
Disconnected changes reconcile on reconnect. A throwing callback is not retried;
reconnecting resumes other pending initial callbacks.

Load buffered definitions through a parser-inserted, non-async ES module script
or a classic `defer` script. Descendants must not structurally mutate a
containing WebUI component's SSR subtree before it hydrates - insertion,
removal, or reordering shifts compiled element indices.

### Progressive streaming hydration

Streaming is an opt-in host integration, not required for ordinary SSR.
`<boundary>` declares a named checkpoint; boundaries cannot nest or occur
inside repeats. Never author generated `<webui-hydrate>` elements or construct
browser streaming records yourself.

Before adding streaming, read the
[`<boundary>` reference](/learn/directives/boundary) for placement and keys,
and [progressive streaming hydration](/build/hydration#progressive-streaming-hydration)
for the browser coordinator and host lifecycle. The host must honor ordering,
backpressure, and cancellation; bytes already streamed cannot be rewound.

### Lazy component mounting

This is the **only** sanctioned exception for inserting UI from JavaScript:
mounting a lazily loaded component through the component-asset API into an
explicit `w-ref` slot. It does not authorize manually building
ordinary UI or removing a child owned by a parent template.

Prefer the template-driven form, which needs no DOM insertion at all:

```typescript
await router.ensureLoaded('settings-dialog');
this.showSettings = true;
```

With the framework loaded, `ensureLoaded()` also waits for stylesheet cache
warming or a native-link fallback decision. If the applying stylesheet fails,
WebUI reports it and completes hydration rather than keeping the component
hidden; the component may be unstyled. See
[on-demand loading](/build/routing#on-demand-component-loading).

```html
<if condition="showSettings">
  <settings-dialog @close="{onCloseSettings()}"></settings-dialog>
</if>
```

Without delivery, follow [static component assets](/learn/interactivity#loading-static-component-assets)
to prebuild and import a generated root, register it with
`defineComponentAsset()`, await `asset.create()`, and mount into an explicit
`w-ref` slot. Load the normal entry first. Component assets cannot be combined
with `<route>`; use the router for routed components.

The bundler owns code splitting and asset URLs. Do not invent hashed stylesheet
paths or edit generated modules. Real open failures need visible errors; resetting
a promise does not guarantee retrying a failed native ESM import.

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
  styles remain deferred until navigation. Matched route CSS targeting the
  Document is applied before `</head>`. ShadowRoot-targeted Link CSS is preloaded
  from the head and applied inside its owning root. Static request-reachable
  Shadow roots are preloaded the same way.

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
import { WebUIRouter } from '@microsoft/webui-delivery/router.js';

const router = new WebUIRouter();
router.start({
  loaders: {
    'home-page': () => import('./pages/home-page.js'),
    'user-detail': () => import('./pages/user-detail.js'),
  },
});
```

The default is JSON-only. Configure selected policies once in `start()`:
`transport: ndjson()` from `resources.js`; `cache: navigationCache(options)`,
`actions: routeActions()`, `preload: routePreload()`,
`boundaries: routeBoundaries`, and `transitions: viewTransitions` from
`router.js`. Preload requires a configured cache. There is no singleton or
second constructor configuration. Do not mutate configuration while running;
destroy before restarting.
See [selecting router capabilities](/build/routing#selecting-router-capabilities)
for the import/configuration pairs.

Route loaders (`static loader({ params, query, signal })`) and actions
(`static action({ formData, params, signal })`, enabled by
`router.start({ actions: routeActions() })`) live on the component class.
Unselected policies are excluded by tree-shaking. Selected event listeners
are ready at startup; route/component behavior modules remain lazy imports.

Every route intended for partial navigation must register a custom element.
Scriptless templates are registered by the compiler-owned host runtime. If a
route remains unregistered after template publication and loader resolution, the
router navigates the document so the server can render it.

Full detail: [Routing](/build/routing).

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

**Missing paths:** text bindings render empty. In conditions, a missing
identifier is falsy, so `path` is false and `!path` is true. No error.

**Route-scoped state.** Each route handler should return only the keys that
route's template binds to. Sending full app state on every route wastes
bandwidth and render time.

### Reserved `$webui` state

The top-level `"$webui"` object is reserved for trusted host metadata: HTML
emitted at document boundaries and the response's asset base:

```json
{
  "$webui": {
    "headEnd": "<link rel=\"preload\" as=\"image\" href=\"/hero.avif\">",
    "bodyStart": "<!-- immediately after <body> -->",
    "bodyEnd": "<script src=\"/livereload.js\"></script>",
    "assetBase": "https://cdn.example.com/b123/"
  }
}
```

All members are optional strings. `headEnd`, `bodyStart`, and `bodyEnd`
are emitted raw immediately before `</head>`, after `<body>`, and before
`</body>`, respectively. Missing, empty, `null`, or non-string HTML members are
ignored. `assetBase` overrides the build's `--asset-base` for this response
(see [CLI reference](/reference/cli/)); `null` keeps the build value, and an
invalid value fails the render. WebUI strips the reserved object from hydration
and partial-navigation state, so client-side templates and code cannot read it.
Asset-base checks cover basic form and character safety, not complete HTTP
hostname or port validation. Supply a valid deployment URL.
The response's asset base is the exception: the rendered head carries it as
`<meta name="webui-asset-base" content="...">`, so an entry module can set its
bundler's public path from it.
Compiler-emitted FAST 2/3 Link stylesheets follow that asset base for SSR and
client-created components, including templates loaded through WebUI delivery.
Authored links keep their authored URLs.

**Never put request-derived or otherwise untrusted content in `$webui`.** The
values are not escaped and can create an XSS vulnerability. See
[Integrations](/build/integrations/) for host-specific rendering details.

### Truthiness

| Value | Truthy? |
|---|---|
| `true` | Yes |
| `false` | No |
| `0` | No |
| Non-zero number | Yes |
| `""` (empty string) | No |
| `"false"` (string!) | **Yes** (non-empty string) |
| `[]` (empty array) | No |
| `{}` (empty object) | No |
| Nonempty array or object | Yes |
| `null` / missing key | No |

**Never use the string `"false"` for boolean state. Use real booleans.**

These rules apply on both server and client. `<if condition="items">` and
`<if condition="items.length">` both test whether an array is nonempty.
Nested paths such as `<if condition="actions.trailing">` also work.
Use `.length` for count comparisons such as `<if condition="items.length > 1">`.

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
      except the documented component-asset mount into an explicit `w-ref` slot.
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
- [ ] Every native `<slot>` resolves to Shadow DOM; in a `--dom light` build its
      component authors a sole top-level `<template shadowrootmode="open">`.
- [ ] A sole bare top-level `<template>` is an explicit Light wrapper and is
      unwrapped even when the build fallback is Shadow.

## Build and run

Building WebUI or a Rust host from source requires Rust 1.99.1 or newer.
Prebuilt npm binaries do not require Rust.

Keep platform optional dependencies enabled. Package managers can block
WebUI's CLI setup script: follow the
[npm, pnpm, Yarn, or Bun approval steps](/start/installation#installation-policies)
before running it. Approve only reviewed packages, including separately
installed build tools when needed. Script approval and registry download
permissions are different; do not add blanket `--allow-remote=all` or
all-scripts overrides to generated install commands.

### Web apps

For an interactive app, install the CLI and browser framework, then bundle the
browser entry into `dist` before serving. A scriptless app can omit the framework
dependency and `--servedir` when it has no static assets.

```bash
# Install the CLI and optional browser framework
npm install @microsoft/webui @microsoft/webui-framework

# Dev server with live reload
webui serve ./src --state ./data/state.json --servedir ./dist --plugin=webui --watch

# Production build
webui build ./src --out ./dist --plugin=webui

# Inspect the compiled protocol
webui inspect ./dist/protocol.bin
```

`webui serve` listens on IPv4 loopback on a nonzero `--port` (port 0 is
rejected). It accepts `127.0.0.1`, `localhost`, and subdomains such as
`play.xbox.localhost` on its port. For a reverse proxy that forwards a custom
Host, add an exact `--allowed-host dev.example[:port]` (repeatable); only add
names you control.
Other Hosts are rejected before the app or API proxy handles them. See the
[serve CLI reference](./reference/cli/#webui-serve) for details.

Use the [CLI reference](/reference/cli/) for CSS/DOM modes, component discovery,
asset bases, generated assets, and shutdown options rather than guessing flags.
For generated modules plus state projection, follow the
[single-compilation build pipeline](/build/integrations/node#single-compilation-builds).
Projection is optional; rebuild its manifest whenever client inputs change.

Authoring mistakes fail the build with a structured diagnostic carrying a stable
code, source location, snippet, and a `help:` fix. Branch on the `code`, never
the message. `--format json` emits one JSON object per error on stdout.

### Desktop apps

Desktop is optional; the same UI composition rules apply inside its webview.
Before adding a native host or packaging an app, read the
[desktop guide](/build/desktop) and [desktop CLI](/reference/cli/#webui-desktop).
Use matching package versions and enable only the capabilities needed.

Native host authority is not automatically renderer authority. For an existing
HTTP app, read the [local-server contract](/build/desktop#existing-http-application):
loopback alone is not authentication. For renderer-to-host calls, follow
[message passing](/build/desktop#message-passing) with explicit grants and
document-scoped lifetimes. Check each capability's platform and security limits
in the guide rather than assuming browser access or cross-platform support.

### External component discovery

Native `--components` discovery derives names from `<component-name>.html`,
scanning the package's `components/` directory by default, with no package-root
fallback. A package can select alternate ordinary HTML roots with
`"webui": { "components": ["./src/components"] }` in its `package.json`.
Only the listed directories are scanned; `[]` disables ordinary fallback.
Roots must be readable package-relative directories, including their symlink
targets. Invalid metadata and paths are errors. FAST CEM assets remain separately
resolved. Explicit local paths and the app directory are scanned as given; do
not repeat the app directory in `--components`.
Template/style exports and CEM names are not interpreted by default discovery;
FAST's special metadata-based layouts remain separate.
Npm collection spellings `@scope/*` and `@scope/package/*` are accepted.
Package subpaths and traversal are not package identifiers; use `./components`
or another explicit local path for filesystem discovery.
Import browser registrations through the package's module exports separately.
Catalog script ownership follows matching `.ts`/`.js` siblings per component;
package exports and `.spec.ts` files do not make unrelated scriptless tags
require registration imports or projection entries.

### WebUI Press

Press is the optional Markdown-site tool, not part of ordinary component
authoring. Use `@microsoft/webui-press` at the same version as `@microsoft/webui`.
Before configuring a site, read the [Press reference](/reference/webui-press)
for commands, layouts, page state, named regions, and appearance.

Prefer its built-in configuration and extension points over copying the shell,
patching generated HTML, or duplicating appearance storage and listeners.
Page state is client-facing: namespace application data to avoid reserved keys
and never include secrets.

Keep source-file links such as `[Install](guide/install.md#setup)` in Markdown.
Press maps discovered Markdown destinations to their actual public page routes
during build/serve, including index pages and `nav.source` overrides, without
changing the source files. Relative links start from the Markdown source
directory; root-relative links start at `contentDir`. Query strings, fragments,
and `basePath` are preserved. See
[Markdown page links](/reference/webui-press#markdown-page-links).

## Server integration

Choose a supported [host binding](/build/integrations/), load `protocol.bin`
once, and reuse it with JSON state per request. Native bindings need a
compatible platform library; WASM is a separate browser-hosted option.

Keep request state, route information, and CSP nonces request-scoped. Follow the
binding's guide for rendering and partial responses rather than translating API
names between languages. With nonce-based CSP, use the same response nonce for
renderer output, authored module scripts, and modulepreload links.

For on-demand routing, implement the
[component template endpoint](/build/routing#on-demand-component-loading).
Streaming additionally requires host lifecycle and transport handling; follow
the [streaming guide](/build/hydration#progressive-streaming-hydration), not a
hand-built browser record format.

## Where to read more

| Topic | Page |
|---|---|
| Directives (`if`, `for`, attributes, `route`) | [/learn/directives/](/learn/directives/) |
| Component authoring and interactivity | [/learn/interactivity](/learn/interactivity) |
| Lazy/interaction policy syntax and combinations | [/learn/directives/lazy](/learn/directives/lazy) |
| Hydration lifecycle, projection, and streaming | [/build/hydration](/build/hydration) |
| Best practices and React-habit pitfalls | [/reference/best-practices](/reference/best-practices) |
| Performance strategy and measurement | [/build/performance](/build/performance) |
| Routing, loaders, actions, caching | [/build/routing](/build/routing) |
| Design tokens and theming | [/learn/css-tokens](/learn/css-tokens) |
| CLI flags, diagnostics, exit codes | [/reference/cli/](/reference/cli/) |
| WebUI Press named regions | [/reference/webui-press](/reference/webui-press) |
| Rust, Node, Python, WASM, FFI, Electron, Tauri | [/build/integrations/](/build/integrations/) |
| Render a first page | [/start/first-page/](/start/first-page/) |
| Add a scriptless component | [/start/first-component/](/start/first-component/) |
| Add browser behavior | [/start/add-interactivity/](/start/add-interactivity/) |
