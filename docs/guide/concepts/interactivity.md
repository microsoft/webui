# Interactivity

WebUI uses **Islands Architecture** for client-side interactivity. Each Web Component is a self-contained island with its own HTML template, scoped CSS, and TypeScript behavior. Only components that need interactivity ship JavaScript - everything else stays as static server-rendered HTML.

## Component Files

Every interactive component consists of three separate files. Templates are declarative - no JavaScript mixing.

```
my-counter/
├── my-counter.html   ← Template (structure and bindings)
├── my-counter.css    ← Styles (scoped at build time)
└── my-counter.ts     ← Behavior (TypeScript class)
```

- **HTML** defines what the component renders and where dynamic values appear
- **CSS** styles the component. WebUI scopes it for Light DOM or keeps native
  Shadow scoping for an opted-in Shadow component
- **TypeScript** defines JS-visible reactive properties, event handlers, and component logic

Components that do not need client-side behavior can omit the TypeScript file:

```
product-card/
├── product-card.html
└── product-card.css
```

Create a custom element only for an Interactive Island: event handlers, custom
lifecycle code, imperative methods, or state that TypeScript code reads or
mutates. `@observable` and `@attr` are optional; add them when JavaScript needs
to access the value or when the value is part of the component's public API.

The sibling `.ts` or `.js` file is the authored behavior boundary. With
manifest-enabled projection, only `@observable` and `@attr` fields opt into
initial state hydration; ordinary template roots remain in the trusted SSR DOM.
Without a client module, bindings, conditionals, and loops still render on the
server, but the component contributes no projected keys. If the framework is
loaded, it can later activate the compiled template for browser-applied state or
soft navigation. Without any projection manifest, the server preserves full
state. Add a same-named client module only for events, lifecycle code,
decorators, or imperative APIs. See
[Hydration](/guide/concepts/hydration) for the full contract.

## The Component Class

Every interactive component extends `WebUIElement` and registers itself as a custom element:

```typescript
import { WebUIElement, attr, observable } from '@microsoft/webui-framework';

export class MyCounter extends WebUIElement {
  @attr label = 'Count';
  @observable count = 0;

  increment(): void {
    this.count += 1;
  }
}

MyCounter.define('my-counter');
```

The matching template (`my-counter.html`):

```html
<button @click="{increment()}">
  {{label}}: {{count}}
</button>
```

And scoped styles (`my-counter.css`):

```css
:host {
  display: inline-block;
}

button {
  padding: 0.5rem 1rem;
  font-size: 1rem;
  cursor: pointer;
}
```

## The `<template>` Tag

Every unwrapped component uses Light DOM and renders its content directly into
the component host.

**Light DOM (most components):**
```html
<!-- my-counter.html -->
<button @click="{increment()}">{{label}}: {{count}}</button>
```

The compiler scopes the paired CSS for this Light component.

**Shadow DOM opt-in:**
```html
<!-- todo-app.html -->
<template shadowrootmode="open"
  @toggle-item="{onToggleItem(e)}"
  @delete-item="{onDeleteItem(e)}"
>
  <h1>{{title}}</h1>
  <div class="todo-list">
    <for each="item in items">
      <todo-item id="{{item.id}}" title="{{item.title}}"></todo-item>
    </for>
  </div>
</template>
```

The open template must be the sole top-level element and wrap the complete
component. Use it when the component needs native `<slot>` composition, a
native Shadow boundary, or **root host events** - listeners on the component
root that catch events bubbling up from child components (`@toggle-item`,
`@delete-item` above). `closed`, another value, invalid placement, multiple
wrappers, or extra top-level content fails the build. A native `<slot>` in an
unwrapped component is also a build error.

To reach a root binding from a **child component**, an event must **bubble** and - because it has to cross the child's shadow boundary - be **composed**. `this.$emit()` sets both whenever the emitting component has a shadow root, which is the default. A hand-built `new CustomEvent('my-event')` defaults to neither and will never arrive - bind it on the child element instead, or pass `{ bubbles: true, composed: true }` yourself.

A root binding lives on the **host element**, so it also catches events targeted at the host itself - what host-interactive components (host `tabindex`, presentational shadow content) rely on. It does not see non-composed events (`change`, `submit`, `select`, media events), which stop at the shadow boundary because they identify one specific inner element; bind those per element - `<input @change="{onChange(e)}">`.

Since the listener is on the host, `e.target` is the host for events raised inside the shadow tree. Use `e.composedPath()[0]` to get the element that was actually hit.

Decorators define how properties behave and how they connect to the template.

### `@attr` - HTML Attributes

Use `@attr` for values passed from a parent element via HTML attributes. These are part of the component's public API.

**String mode** (default):

```typescript
@attr label = 'Default Label';
```

```html
<my-counter label="Items"></my-counter>
```

**Boolean mode** - attribute presence means `true`, absence means `false`:

```typescript
@attr({ mode: 'boolean' }) disabled = false;
```

```html
<!-- disabled = true -->
<my-button disabled></my-button>

<!-- disabled = false -->
<my-button></my-button>
```

When build-time projection is enabled, `@attr` property names are included in
initial state metadata. If SSR already emitted the corresponding host
attribute, that attribute is authoritative during hydration. Projected state
fills the property only when the host attribute is absent.

### `@observable` - Reactive State

Use `@observable` for internal state that changes over time. When an observable value changes, the framework automatically updates any template bindings that reference it.

```typescript
@observable count = 0;
@observable items: Item[] = [];
@observable isOpen = false;
```

Observable changes are **synchronous and targeted** - only the specific DOM nodes bound to the changed property are updated.

You do not need `@observable` for values that are only read by the template.
Add `@observable` when TypeScript code needs to read or mutate the value, for
example in an event handler.

### Initial Hydration State

For the initial page, build-time projection can narrow state to top-level
`@observable` and `@attr` values from authored components. Template bindings
still render on the server, but they do not automatically become JavaScript
state. Only components reachable on the active route contribute projected
values. Without a projection manifest, WebUI intentionally sends full state.

See [Hydration](/guide/concepts/hydration) for HTML-only components, soft
navigation, and payload behavior.

### Derived State

For derived values like "has items?" or "total count", use template expressions directly instead of computed properties:

```html
<!-- Use dot-path expressions in the template -->
<if condition="items.length">
  <span>{{items.length}} items</span>
</if>
```

The condition evaluator supports dot paths (`items.length`), comparisons (`count > 0`), truthiness, and negation (`!isEmpty`). This keeps derived state declarative and works on both server and client.

For complex derived state that can't be expressed in template syntax, compute it on the server and provide it in the JSON state, or compute it in an event handler and store it in an `@observable`.

## Template Syntax for Interactivity

### Reactive Text

Use double curly braces to bind property values into the template:

```html
<span>{{label}}: {{count}}</span>
<p>Hello, {{user.name}}!</p>
```

### Event Binding

Attach event handlers with `@event` syntax:

```html
<!-- Call a method -->
<button @click="{increment()}">Add</button>

<!-- Access the event object -->
<input @keydown="{onKeydown(e)}" />

<!-- Pass repeat-scope values and literals -->
<for each="item in items">
  <button @click="{selectItem(item.id, 'details', e)}">
    {{item.name}}
  </button>
</for>

<!-- Multiple events on one element -->
<div @mouseenter="{onHover()}" @mouseleave="{onLeave()}">
  Hover me
</div>
```

Components that use `@event` must have authored `.ts` or `.js` code that
defines a `WebUIElement` for the tag. HTML-only components do not provide
application event handlers.

Event handlers use method-call syntax only. Arguments can be:

- `e` for the native DOM event
- Dotted component or repeat-scope paths such as `item.id`
- String, number, boolean, and `null` literals

General JavaScript expressions and nested function calls are not parsed in
templates. Compute those values in the component class or pass a supported path.

Invalid handler syntax, such as a general expression like
`@click="e.preventDefault()"` or a bare name like `@click="{closeMenu}"`, fails
the build with an actionable
error that names the offending component and element.

### DOM References

Use `w-ref` to get a direct reference to a DOM element:

```html
<input w-ref="{inputEl}" type="text" />
<button @click="{focusInput()}">Focus</button>
```

```typescript
inputEl!: HTMLInputElement;

focusInput(): void {
  this.inputEl.focus();
}
```

`w-ref` must use braces to bind to a component property: `w-ref="{inputEl}"`
(or the unquoted `w-ref={inputEl}`), never `w-ref="inputEl"`. The build fails
with an actionable error otherwise.

### Conditional Rendering

Render content based on expressions:

```html
<if condition="count > 0">
  <p>You have {{count}} items.</p>
</if>

<if condition="!isLoggedIn">
  <a href="/login">Sign in</a>
</if>
```

### Boolean Attributes

Toggle HTML attributes with the `?` prefix:

```html
<button ?disabled="{{isLoading}}">Submit</button>
<input ?checked="{{isSelected}}" type="checkbox" />
<details ?open="{{isExpanded}}">...</details>
```

### Property Bindings

Use the `:` prefix to pass rich values directly to child DOM properties:

```html
<profile-card :config="{{settings}}"></profile-card>
```

For client-created component trees, WebUI applies initial property bindings before child `connectedCallback` methods run. This lets a child read a parent-provided property during setup. If the parent has not provided a value, the child can initialize a fallback in `connectedCallback`; later parent updates still flow through the live binding.

### List Rendering

Iterate over arrays with `<for>`:

```html
<ul>
  <for each="item in items">
    <li>{{item.name}} - {{item.price}}</li>
  </for>
</ul>
```

## Event Handling Patterns

### Direct Method Calls

The simplest pattern - call a method when an event fires:

```typescript
@observable count = 0;

increment(): void {
  this.count += 1;
}
```

```html
<button @click="{increment()}">+1</button>
```

### Using the Event Object

Access the native DOM event by passing `e`:

```typescript
onKeydown(e: KeyboardEvent): void {
  if (e.key === 'Enter') {
    this.submit();
  }
}
```

```html
<input @keydown="{onKeydown(e)}" />
```

### Passing Values from Repeats

Handlers inside a `<for>` block can receive the current item through a dotted
path. The framework captures the active repeat scope during hydration and
resolves the argument when the event fires:

```typescript
selectItem(id: string, e: MouseEvent): void {
  e.preventDefault();
  this.selectedId = id;
}
```

```html
<for each="item in items">
  <button @click="{selectItem(item.id, e)}">
    {{item.title}}
  </button>
</for>
```

### How Event Bindings Are Wired

Every `@event` gets its own listener on the element it is written on. Bindings
are never delegated to a shared root listener, which has two consequences worth
knowing:

- **Non-bubbling events work.** `@focus`, `@blur`, `@mouseenter`, `@load`,
  `@error`, and `@toggle` fire normally. A shared root listener could never see
  them, because those events do not travel up the tree.
- **`stopPropagation()` behaves as written.** An ancestor stopping an event as it
  bubbles cannot suppress a handler bound to the element the event started on.

Dispatch cost does not grow with the number of rows a `<for>` renders. If you do
want a single listener instead of one per row for a very long list, use root host
events and find the row yourself with `e.composedPath()`.

### Custom Events and Parent-Child Communication

Components communicate upward by emitting custom events with `this.$emit()`:

**Child component** (`color-picker.ts`):

```typescript
export class ColorPicker extends WebUIElement {
  @observable selectedColor = '';

  selectColor(color: string): void {
    this.selectedColor = color;
    this.$emit('color-change', { color });
  }
}
```

**Parent template** catches the event:

```html
<color-picker @color-change="{onColorChange(e)}"></color-picker>
<p>Selected: {{currentColor}}</p>
```

**Parent class** handles the event:

```typescript
onColorChange(e: CustomEvent): void {
  this.currentColor = e.detail.color;
}
```

This pattern keeps components decoupled - the child doesn't know who is listening, and the parent reacts declaratively.

## Loading Static Component Assets

When you are not using `@microsoft/webui-router`, deferred UI can still be
loaded from static files. Build the root components as assets:

```bash
webui build ./src --out ./dist --plugin=webui \
  --emit-component-assets settings-dialog,mail-thread
```

Each requested root writes an ESM graph module such as `<tag>.webui.js` next to
`protocol.bin`. Entry-reachable dependencies remain in the application bundle
and protocol. Dependencies used by one requested root stay inline, while
dependencies used by the same set of two or more roots are emitted once as
`chunk-<component>.webui.js` and dynamically imported by those roots. Asset-only
fragments and component records do not remain in `protocol.bin`.

Static component assets and `<route>` cannot be used in the same build. Use
`@microsoft/webui-router` for routed components, or use component assets for
non-routed deferred UI.

During development, pass the same flag to `webui serve` so these roots are
validated and served without a separate build step:

```bash
webui serve ./src --state ./data/state.json --plugin=webui \
  --emit-component-assets settings-dialog,mail-thread \
  --metafile ./component-assets-meta.json --watch
```

The dev server parses and validates each root on every build. HTML and
theme-token errors in a lazily loaded component fail the build instead of being
missed because the component is outside the initial SSR tree. The dev server
serves roots and shared chunks from memory and rebuilds them on change. A
successful watch build atomically replaces `--metafile`; a failed build
preserves the previous valid graph.

Load the asset before creating or revealing the component:

```typescript
import { WebUIElement } from '@microsoft/webui-framework';
import { settingsAssets } from './lazy-assets.js';

export class AppShell extends WebUIElement {
  panelSlot!: HTMLDivElement;

  async openSettings(): Promise<void> {
    settingsAssets.preload('settings-dialog');
    this.panelSlot.replaceChildren(await settingsAssets.create('settings-dialog'));
  }
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

`defineComponentAssets()` exposes `preload(tag)` and `create(tag)`. Keep only
stable root inputs in the manifest; generated chunk filenames can change as the
dependency graph changes, and each root asset already carries its dynamic
imports. `preload(tag)` starts the component's template graph, styles,
JavaScript module, and optional data together. Components can then fetch their
own data in their class code and expose it through `@observable` fields when
JavaScript needs to read or mutate it. Concurrent roots deduplicate shared chunk
imports by resolved URL.
`create(tag)` creates the element after template/module work is ready. Use
`create(tag, { awaitData: true, dataTimeoutMs: 150 })` only when a component must
wait briefly for state before mounting. Use a manifest helper when you want the
fastest path: it lets the shell start the template asset, JS chunk, and data
fetch in parallel.

Do not put `<settings-dialog>` in an SSR-reachable `<if>` block for this pattern.
If the server state ever makes that condition true, the component is part of the
initial SSR graph instead of being loaded only from the static asset. Always
load the normal application entry bundle first; component assets treat all
entry-reachable templates as external prerequisites and fail clearly when one
is missing.

## Styling

Keep styles in the ordinary paired `.css` file. For an unwrapped Light
component, the compiler scopes rules to that component, lowers `:host`, and
namespaces static keyframes. For authored Shadow roots, the browser provides
native style scoping.

### The `:host` Selector

Style the component's root element with `:host`:

```css
:host {
  display: block;
  padding: 1rem;
  border: 1px solid #e0e0e0;
}
```

### Attribute-Based Styling

Style the component differently based on its attributes with `:host([attr])`:

```css
:host([variant="primary"]) {
  background: #0078d4;
  color: white;
}

:host([disabled]) {
  opacity: 0.5;
  pointer-events: none;
}
```

### Scoping Rules

- Paired component styles are scoped for both Light and Shadow components
- `:host` and `:host([attr])` work in both forms
- Light DOM keeps normal inheritance and cascade behavior
- Shadow DOM provides a native style boundary
- Shadow-only selectors and unsafe dynamic local-keyframe references in Light
  components fail the build
- No CSS-in-JS - styles stay in `.css` files, separate from behavior
- Use CSS custom properties (`--my-color`) to allow external theming

## SSR + Interactivity Lifecycle

Understanding the lifecycle helps you write components that work correctly from the first paint through interactive use.

### 1. Server renders HTML

The handler renders the component's template using JSON state data. No
JavaScript runs. For an unwrapped component, content is emitted directly:

```html
<my-counter>
  <button>Count: 0</button>
</my-counter>
```

An opted-in Shadow component instead emits Declarative Shadow DOM with its
styles installed inside the shadow root.

### 2. Browser displays content

The browser parses the HTML and renders it immediately. The user sees fully styled content - no loading spinner, no blank page, no flash of unstyled content.

### 3. JavaScript loads and components hydrate

The framework detects the existing Light or Shadow SSR DOM and upgrades
elements in place:

- Bindings are wired to class properties
- Event handlers are attached
- `@observable` properties become reactive
- Existing host attributes take precedence over projected `@attr` state
- The component is now interactive

### 4. User interacts

From this point on, interactions are handled entirely on the client. Changes to `@observable` properties trigger targeted DOM updates without a server round-trip.

### Setting observable state during setup

The server owns the first paint, and the framework **trusts** the HTML it
produced. Hydration wires bindings to the existing DOM instead of re-rendering
it. A value you write *before hydration finishes*, in an `@observable` field
initializer, the `constructor`, or before you call `super.connectedCallback()`,
updates the property's backing field but cannot touch the DOM yet, so it is
dropped. Your element's state then silently disagrees with what is on screen.

When the framework detects this it logs a development warning naming the properties, so the mismatch is never silent:

```
[WebUI] Hydration mismatch on <my-counter>: "count" changed at or before
super.connectedCallback() to a value that differs from the server-rendered DOM…
```

Follow these rules to stay correct:

- **A value that must appear in the first render belongs in the SSR state.** Provide it in the JSON state so the server renders it; the client then hydrates against a matching DOM.
- **Assign post-hydration state from `hydratedCallback()`**, where
  `@observable` writes flow through live bindings on ordinary, streamed, and
  client-created components.

On a normal buffered page or client-created mount,
`super.connectedCallback()` hydrates synchronously. A progressive streaming host
can connect while its `data-ws` boundary is incomplete, however, so the same call
returns while hydration is still deferred. `hydratedCallback()` is the
cross-mode lifecycle: WebUI invokes it synchronously exactly once after the
first successful hydration or mount. Its once-latch is set before author code,
so reconnecting the element or throwing from the callback does not retry it.

Buffered pages rely on loading authored component code with a parser-inserted,
non-async ES module script or a classic `defer` script. If a classic script
blocks parsing, place it after every SSR instance it may upgrade. An opt-in
[progressive streaming page](/guide/concepts/hydration#progressive-streaming-hydration)
instead loads an early async module and gates each component until its complete
streaming boundary commits.

Descendants must not structurally mutate a containing WebUI component's SSR
subtree before that component hydrates. Inserting, removing, or reordering nodes
can shift compiled element indices before WebUI wires them.

```ts
export class MyCounter extends WebUIElement {
  @observable count = 0;

  constructor() {
    super();
    // ✗ Wrong: this runs before hydration, so the write is dropped and warns.
    // this.count = 3;
  }

  protected override hydratedCallback(): void {
    // ✓ Correct: this hook runs after hydration in every mode.
    this.count = 3;
  }
}
```

If `count` should already read `3` in the server-rendered HTML, seed it in the SSR state instead of assigning it on the client at all.

#### Development warning

The mismatch warning is removed from `webui-press` production builds. If you
bundle the framework yourself, define `__WEBUI_DEV__` as `false` in production:

```bash
esbuild app.ts --bundle --minify --define:__WEBUI_DEV__=false
```


## When NOT to Hydrate

Not every component needs JavaScript. Hydrating a component that has no interactivity adds unnecessary bytes and processing time.

**Skip hydration for:**

- **Static content pages** - about, docs, marketing, legal. The server renders them perfectly.
- **Read-only data displays** - lists, tables, cards with no user interaction. Server-rendered HTML is sufficient.
- **Layout components** - headers, footers, sidebars with only links. Standard `<a>` tags work without JS.

**Hydrate when a component needs:**

- Event handlers (`@click`, `@keydown`, `@input`)
- Reactive state updates (`@observable` properties that change)
- User input handling (forms, search, filters)
- Client-side data manipulation (sorting, filtering, pagination)

The goal is minimal JavaScript: hydrate only what the user will interact with, and let the server handle everything else.
