---
name: webui-dev
description: Build interactive WebUI example apps with compiled-template hydration, template syntax, and component patterns.
---

# WebUI App Development

Use this skill when building or modifying example apps under `examples/app/`.

WebUI server-renders HTML at request time. The `@microsoft/webui-framework` runtime hydrates the pre-rendered DOM on the client using compiled template metadata and direct DOM binding — no virtual DOM, no runtime template parsing.

## Project structure

Every example app follows this layout:

```
examples/app/<name>/
├── src/
│   ├── index.html              # HTML shell + CSS design tokens
│   ├── index.ts                # Component registration
│   └── <component-name>/      # One directory per component
│       ├── <component-name>.ts
│       ├── <component-name>.html
│       └── <component-name>.css
├── data/
│   └── state.json              # Initial state for SSR
├── package.json
└── tsconfig.json
```

### package.json

```json
{
  "type": "module",
  "scripts": {
    "start:client": "esbuild src/index.ts --bundle --outfile=dist/index.js --format=esm --sourcemap --watch",
    "start:server": "cargo run -p microsoft-webui-cli -- start ./src --state ./data/state.json --plugin=webui --servedir ./dist --port 3001 --watch",
    "start": "cargo xtask dev <name>"
  },
  "devDependencies": {
    "@microsoft/webui-framework": "workspace:*",
    "esbuild": "catalog:",
    "typescript": "catalog:"
  }
}
```

Use `catalog:` for dependency versions — they resolve from `pnpm-workspace.yaml`.

### tsconfig.json

Required settings:

```json
{
  "compilerOptions": {
    "experimentalDecorators": true,
    "useDefineForClassFields": false
  }
}
```

`useDefineForClassFields: false` is **mandatory** — decorators rely on legacy class field behavior.

## Template syntax (HTML)

### Signal interpolation — `{{}}`

Binds server-side state values into HTML. Processed by the WebUI parser at build time.

```html
<!-- Text content -->
<p>{{userName}}</p>

<!-- Attributes -->
<a href="{{url}}">{{linkText}}</a>

<!-- Nested properties -->
<span>{{user.profile.name}}</span>

<!-- Inside loops (item properties) -->
<for each="item in items">
  <div>{{item.title}}</div>
</for>

<!-- Inline styles -->
<div style="background-color: {{color}}">...</div>
```

### Raw (unescaped) interpolation — `{{{}}}`

Passes HTML through without entity encoding:

```html
<div>{{{rawHtmlContent}}}</div>
```

### Loop rendering — `<for>`

```html
<for each="item in items">
  <my-component
    name="{{item.name}}"
    value="{{item.value}}"
  ></my-component>
</for>
```

- Loop variable is scoped to the `<for>` block and its children.
- Nested loops can access outer loop items via their moniker.
- Array length: `{{items.length}}`. Array indexing: `{{items.0.name}}`.

### Conditional rendering — `<if>`

```html
<if condition="isVisible">
  <div>Shown when truthy</div>
</if>

<if condition="count > 10">
  <div>Comparisons work</div>
</if>

<if condition="isActive && role == 'admin'">
  <div>Logical operators work</div>
</if>
```

Supported: `==`, `!=`, `>`, `<`, `>=`, `<=`, `&&`, `||`, `!`. Max 5 operators per expression.

### Boolean attributes — `?`

Rendered only when the bound value is truthy:

```html
<input ?disabled={{isDisabled}} />
<div ?hidden={{!isVisible}}>...</div>
```

### Complex (pass-through) attributes — `:`

For structured data that passes through as-is:

```html
<my-component :config={{settings}}></my-component>
```

## Component patterns

### Component class

Every component extends `WebUIElement`:

```typescript
import { WebUIElement, attr, observable, volatile } from '@microsoft/webui-framework';

export class MyComponent extends WebUIElement {
  @attr label = 'Default';           // HTML attribute (string, reflected)
  @observable count = 0;             // Reactive property
  @observable items: Item[] = [];    // Collection for @for loops

  @volatile get doubled(): number {  // Computed, re-evaluated on access
    return this.count * 2;
  }

  increment(): void {
    this.count += 1;
  }
}

MyComponent.define('my-component');
```

### `@attr` — HTML attributes

- Reflected to/from the HTML attribute on the element.
- **Use `!:` (no initializer)** for fields seeded from SSR. Class field initializers run AFTER `super()`, overwriting values set during hydration.
- Hyphenated HTML attributes map via `@attr({ attribute: 'display-value' })`.
- Attribute values arrive as strings — use `@observable` for non-string state.

### `@observable` — Reactive properties

- Triggers per-path targeted update when changed — only bindings referencing this property are visited.
- Use for data that drives `<for>` loops, `<if>` conditionals, or text/attribute bindings.
- **Use `!:` (no initializer)** for fields seeded from SSR.

### `@volatile` — Computed getters

- Re-evaluated on every access (no caching).
- Automatically included in targeted updates via a wildcard binding.

### `$emit` — Custom events

```typescript
this.$emit('toggle-item', { id: this.id });
```

Events bubble through the shadow DOM boundary via `composed: true`.

### `w-ref` — Template refs

Bind a DOM element to a component property for direct access:

```html
<input w-ref="myInput" @keydown="{onKeydown(e)}" />
```

```typescript
myInput!: HTMLInputElement;  // Populated during hydration

onSubmit(): void {
  const value = this.myInput.value;
  this.myInput.value = '';
  this.myInput.focus();
}
```

Use refs only for DOM-only concerns (focus, measurement, selection). Application state belongs in `@observable` / `@attr`.

## Event handling

### Template event binding — `@event`

```html
<!-- Standard DOM events -->
<button @click="{onClick()}">Click</button>
<input @keydown="{onKeydown(e)}" />

<!-- Custom events (bubble up from children) -->
<template shadowrootmode="open"
  @toggle-item="{onToggleItem(e)}"
  @delete-item="{onDeleteItem(e)}"
>
```

Pass `e` to receive the event object. Omit it when not needed.

Events use delegation internally — one listener per event type on the shadow root, not one closure per element.

## Component template (HTML)

The `<template shadowrootmode="open">` wrapper is **optional** — the framework adds it automatically when absent. Only include it when you need root-level event listeners:

```html
<!-- Minimal — no root-level bindings needed -->
<div>{{title}}</div>
<for each="item in items">
  <child-component name="{{item.name}}"></child-component>
</for>

<!-- With root-level event listeners — wrapper required -->
<template shadowrootmode="open"
  @custom-event="{onCustomEvent(e)}"
>
  <div>{{title}}</div>
</template>
```

## Component styles (CSS)

Scoped to the component via shadow DOM:

```css
:host {
  display: block;
}

:host([state="active"]) .indicator {
  background: var(--color-accent);
}

.content {
  padding: var(--spacing-m);
}
```

Use `:host([attr="value"])` selectors to style based on attribute state.

## SSR hydration

The framework handles two paths automatically:

**SSR path** — the server renders a Declarative Shadow Root with hydration markers. On custom element upgrade, the framework:
1. Walks SSR markers once to connect bindings
2. Seeds `@observable` / `@attr` values from DOM content inline
3. Removes markers
4. DOM is already correct — no `$update()` needed

**Client-created path** — for components created dynamically (e.g. inside `@for` loops):
1. Clones cached template HTML (`cloneNode`, not `innerHTML`)
2. Resolves binding locators from compiled metadata
3. Calls `$update()` to flush initial state

### State seeding

The framework automatically reconstructs `@observable` state from SSR DOM:
- `@observable count = 0` + SSR text `"42"` → `this.count = 42` (coerced to number)
- `@observable active = false` + SSR attr `"true"` → `this.active = true` (coerced to boolean)

Do NOT manually read DOM values in `connectedCallback` — the framework does it for you.

## State (data/state.json)

Provides initial values for SSR template rendering:

```json
{
  "textdirection": "ltr",
  "language": "en",
  "title": "My App",
  "items": [
    { "id": "1", "name": "First", "status": "active" }
  ]
}
```

Top-level keys become template signals (`{{title}}`). Arrays drive `<for>` loops.

## CSS design tokens

Define design tokens as CSS custom properties in `index.html`:

```html
<style>
  :root {
    --color-brand-primary: #0078d4;
    --spacing-m: 12px;
    --border-radius-m: 6px;
    --font-family-base: 'Segoe UI', sans-serif;
  }
</style>
```

Components reference tokens via `var(--token-name)`.

## Dev workflow

```bash
# Run dev server (builds + serves + watches)
cargo xtask dev <app-name>

# Or run client and server separately:
pnpm start:client    # esbuild watch
pnpm start:server    # microsoft-webui-cli dev server

# Production build
webui build ./src --out ./dist --plugin=webui
```

The `cargo xtask dev` command auto-discovers apps from `examples/app/` — no registration needed.
