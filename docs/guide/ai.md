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
   If your template uses <code v-pre>{{title}}</code>, the server must provide
   `{ "title": "..." }`.

2. **Derived state belongs in the server or the template.** Use template
   expressions like `items.length` or `status == 'active'` for simple
   derivations. For complex values, compute on the server.

3. **The server is the source of truth for the initial render.** The client
   takes over after hydration for user interactions.

4. **Static content never ships JavaScript.** Only components with event
   handlers, reactive state, or user input need client-side code.

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
├── data/
│   └── state.json              ← Server state for dev
└── package.json
```

**Component discovery rules:**
- HTML files with a hyphen in the name are components (`my-card.html` → `<my-card>`)
- CSS files with the same name are auto-paired (`my-card.css`)
- TypeScript files provide client-side behavior (`my-card.ts`)
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

- <code v-pre>{{expr}}</code> - HTML-escaped output (safe for user input)
- <code v-pre>{{{expr}}}</code> - raw/unescaped output (only for trusted content)

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

<!-- Mixed static + dynamic -->
<img src="/img/{{user.avatar}}" alt="{{user.name}}" />

<!-- Complex/property binding -->
<my-widget :config="{{settings}}"></my-widget>
```

### Events (client-side only)

```html
<button @click="{handleClick()}">Click me</button>
<input @keydown="{onKeydown(e)}" />
<div @mouseenter="{onHover()}" @mouseleave="{onLeave()}">Hover</div>
```

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

- Child paths are relative to parent (no leading `/`)
- Use `exact` on leaf routes (no children)
- Omit `exact` on parent routes that have `<outlet />`
- Path params: `:id` (required), `:query?` (optional), `*path` (catch-all)

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

### Decorator reference

| Decorator | Purpose | SSR? | Triggers DOM update? |
|-----------|---------|------|---------------------|
| `@attr` | HTML attribute reflection | Yes (from JSON state) | Yes |
| `@attr({ mode: 'boolean' })` | Boolean attribute (present/absent) | Yes | Yes |
| `@observable` | Reactive internal state | Yes (from JSON state) | Yes |


### Component API

| Method/Property | Description |
|----------------|-------------|
| `this.$emit(name, detail?)` | Dispatch a CustomEvent that bubbles up |
| `this.$update()` | Force a reactive update cycle |
| `this.$flushUpdates()` | Synchronously flush pending updates |
| `setInitialState(state, params?)` | Populate from router navigation state |
| `static define(tagName)` | Register as a custom element |

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
- Use CSS custom properties (`var(--name)`) for theming
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
| `--plugin <NAME>` | none | `webui` or `fast` |
| `--components <PACKAGE>` | none | Extra component sources (repeatable) |

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
| `--plugin <NAME>` | none | `webui` or `fast` |
| `--components <PACKAGE>` | none | Extra component sources (repeatable) |
| `--api-port <PORT>` | none | Proxy route requests to API server |
| `--theme <PACKAGE>` | none | Design token theme (see below) |

### Inspect

```bash
webui inspect ./dist/protocol.bin
webui inspect ./dist/protocol.bin | jq '.fragments | keys'
```

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

**Local path scanning** works like app directory scanning: HTML files with
hyphenated names are registered as components, matching CSS files are
auto-paired.

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

The `--theme` flag loads a token JSON file and injects resolved CSS custom
property declarations into the render state. Only available on `webui serve`.

**What it accepts:**

- **Local JSON file** - `./themes/dark.json`
- **npm package** - `@my-org/brand-tokens` (looks for `tokens.json` inside the package)
- **npm package with subpath** - `@my-org/brand-tokens/custom.json`

**How it works:**

1. Loads the JSON file (multi-theme or flat single-theme format)
2. Filters tokens to only those actually used in your CSS (`var(--name)`)
3. Expands transitive `var()` references and detects cycles
4. Generates CSS declaration strings per theme
5. Injects into state as `state.tokens.light`, `state.tokens.dark`, etc.

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

**Template usage** - use a comment-based signal to inject tokens:
```html
<style>
  :root {
    <!--{{{tokens.light}}}-->
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

1. **No ternary in templates.** <code v-pre>{{x ? 'yes' : 'no'}}</code> does not work.
   Use `<if>` blocks or boolean attributes instead.

2. **No function calls in bindings.** <code v-pre>{{formatDate(item.date)}}</code> does not
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
   with explicit updates in event handlers.

8. **Components inside `<for>` loops do NOT inherit loop variables.**
   Pass data explicitly via attributes.

9. **No `import` or `require` in templates.** Components are discovered
   by file naming convention, not imports.

10. **No `this.querySelector()` for reactive state.** Use `@observable` and
    template bindings. Use `w-ref` only for imperative DOM access (focus,
    scroll, etc.).

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

Note: `remove(item.id)` does not work as written because template event
handlers cannot pass arguments from loop scope. Instead, use a child
component that emits a custom event:

```html
<for each="item in items">
  <list-item id="{{item.id}}" text="{{item.text}}"
    @remove-item="{onRemove(e)}">
  </list-item>
</for>
```

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

### Derived state (prefer template expressions over shadow observables)

```html
<!-- DO: use expression directly -->
<if condition="items.length">
  <span>{{items.length}} items</span>
</if>

<!-- DON'T: create a shadow observable -->
<!-- @observable hasItems = false; // mirrors items.length > 0 -->
```

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
    "build": "webui build ./src --out ./dist --plugin=webui",
    "dev": "webui serve ./src --state ./data/state.json --plugin=webui --watch"
  },
  "dependencies": {
    "@microsoft/webui": "latest",
    "@microsoft/webui-framework": "latest"
  }
}
```

Add `@microsoft/webui-router` if using client-side navigation.

## Language Integration (Server Side)

WebUI renders from **any** backend. The server loads `protocol.bin` once
and renders with JSON state per request.

### Rust

```rust
let protocol = WebUIProtocol::from_protobuf(&fs::read("dist/protocol.bin")?)?;
let state = json!({ "title": "Home", "items": items_vec });
let mut handler = WebUIHandler::new();
handler.handle(&protocol, &state, &options, &mut writer)?;
```

### Node.js

```javascript
import { render } from '@microsoft/webui';
const protocol = readFileSync('./dist/protocol.bin');
const html = render(protocol, JSON.stringify(state), 'index.html', req.url);
```

### Python (FFI)

```python
ptr = lib.webui_render(html_bytes, json_bytes)
result = ctypes.cast(ptr, c_char_p).value.decode("utf-8")
lib.webui_free(ptr)
```

### Go (cgo)

```go
ptr := C.webui_render(cHTML, cJSON)
defer C.webui_free(ptr)
result := C.GoString(ptr)
```

### C# (P/Invoke)

```csharp
IntPtr ptr = webui_render(html, dataJson);
string result = Marshal.PtrToStringUTF8(ptr);
webui_free(ptr);
```
