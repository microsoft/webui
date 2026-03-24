---
name: webui-dev
description: Build interactive WebUI example apps with FAST-HTML hydration, template syntax, and component patterns.
---

# WebUI App Development with FAST-HTML

Use this skill when building or modifying example apps under `examples/app/`.

WebUI server-renders HTML at request time. FAST-HTML hydrates the pre-rendered DOM on the client, making it interactive without a full re-render.

## Project structure

Every example app follows this layout:

```
examples/app/<name>/
├── src/
│   ├── index.html              # HTML shell + CSS design tokens
│   ├── index.ts                # Hydration bootstrap
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
    "start:server": "cargo run -p microsoft-webui-cli -- start ./src --state ./data/state.json --plugin=fast --servedir ./dist --port 3001 --watch",
    "start": "cargo xtask dev <name>"
  },
  "devDependencies": {
    "@microsoft/fast-element": "catalog:",
    "@microsoft/fast-html": "catalog:",
    "esbuild": "catalog:",
    "tslib": "catalog:",
    "typescript": "catalog:"
  }
}
```

Use `catalog:` for all dependency versions — they resolve from `pnpm-workspace.yaml`.

### tsconfig.json

Required settings for FAST-HTML:

```json
{
  "compilerOptions": {
    "experimentalDecorators": true,
    "useDefineForClassFields": false
  }
}
```

`useDefineForClassFields: false` is **mandatory** — FAST decorators rely on legacy class field behavior.

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

## FAST-HTML component patterns

### Component class

Every component extends `RenderableFASTElement(FASTElement)`:

```typescript
import { FASTElement, attr, observable } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

export class MyComponent extends RenderableFASTElement(FASTElement) {
  @attr name!: string;              // HTML attribute (string, reflected)
  @observable items!: ItemData[];   // Reactive property (triggers re-render)

  inputRef!: HTMLInputElement;      // Template ref target (via f-ref)

  async prepare(): Promise<void> {
    // Hydration hook — read state from pre-rendered DOM
  }

  connectedCallback(): void {
    super.connectedCallback();
    // Post-hydration setup (event listeners, focus, etc.)
  }
}

MyComponent.defineAsync({
  name: 'my-component',
  templateOptions: 'defer-and-hydrate',
});
```

### `@attr` — HTML attributes

- Reflected to/from the HTML attribute on the element.
- **Use `!:` (no initializer)** for fields set in `prepare()`. Class field initializers run AFTER `super()`, overwriting values set during hydration.
- Hyphenated HTML attributes map via `@attr({ attribute: 'display-value' })`.

### `@observable` — Reactive properties

- Triggers FAST template re-rendering when changed.
- Use for data that drives `<for>` loops or conditional display.
- **Use `!:` (no initializer)** for fields set in `prepare()`.

### `prepare()` — Hydration hook

Called during hydration to initialize component state from the pre-rendered DOM. This is **the** place to read server-rendered data:

```typescript
async prepare(): Promise<void> {
  // Read @attr values from HTML attributes (initializers haven't run yet)
  this.mode = this.getAttribute('mode') || 'default';

  // Read child elements rendered by SSR
  const items: ItemData[] = [];
  for (const el of this.shadowRoot!.querySelectorAll('my-item')) {
    items.push({
      id: el.getAttribute('id') || '',
      title: el.getAttribute('title') || '',
    });
  }
  this.items = items;
}
```

Rules:

- Always read `@attr` values from `this.getAttribute()` — decorators initialize AFTER `prepare()`.
- Read child element data from `this.shadowRoot.querySelectorAll()`.
- Guard with `if (!this.shadowRoot) return;` when appropriate.
- Avoid `<for>` loops over `@observable` string arrays — use object arrays or static HTML.

### Component template (HTML)

The `<template shadowrootmode="open">` wrapper is **optional** — the framework adds it automatically when absent. Only include it when you need to attach event listeners or other directives to the template root:

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

### Component styles (CSS)

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

## Event handling

### Template event binding — `@event`

Bind DOM and custom events to component methods in the template:

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

### Emitting custom events — `$emit`

Child components emit events to parent components:

```typescript
// Emit a custom event with detail data
this.$emit('toggle-item', { id: this.id });

// Parent catches it via @toggle-item="{onToggleItem(e)}" on its <template>
```

Events bubble through the shadow DOM boundary via `composed: true`.

### Template refs — `f-ref`

Bind a DOM element to a component property for direct access:

```html
<input f-ref="{myInput}" @keydown="{onKeydown(e)}" />
```

```typescript
myInput!: HTMLInputElement;  // Populated after hydration

onSubmit(): void {
  const value = this.myInput.value;
  this.myInput.value = '';
  this.myInput.focus();
}
```

## Hydration bootstrap (index.ts)

The entry point registers components and activates hydration:

```typescript
performance.mark('app-hydration-started');

import { TemplateElement } from '@microsoft/fast-html';

// Side-effect imports register components
import './my-app/my-app.js';
import './my-item/my-item.js';

TemplateElement.options({
  'my-app': { observerMap: 'all' },
  'my-item': { observerMap: 'all' },
}).config({
  hydrationComplete() {
    performance.measure('app-hydration-completed', 'app-hydration-started');
    console.log('Hydration complete!');
  },
}).define({
  name: 'f-template',
});
```

- List **every** component in `.options()` with `{ observerMap: 'all' }`.
- `.define({ name: 'f-template' })` triggers hydration.
- Performance marks measure hydration time.

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

Top-level keys become template signals (`{{title}}`). Arrays drive `<for>` loops. The state is passed to the WebUI CLI via `--state`.

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
  * { box-sizing: border-box; margin: 0; padding: 0; }
  body { font-family: var(--font-family-base); }
</style>
```

Components reference tokens via `var(--token-name)`. WebUI hoists `var()` usages at build time into the protocol for host-language token resolution.

## Dev workflow

```bash
# Run dev server (builds + serves + watches)
cargo xtask dev <app-name>

# Or run client and server separately:
pnpm start:client    # esbuild watch
pnpm start:server    # microsoft-webui-cli dev server

# Production build
webui build ./src --out ./dist --plugin=fast
```

The `cargo xtask dev` command auto-discovers apps from `examples/app/` — no registration needed.
