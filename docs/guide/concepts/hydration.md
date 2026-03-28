# Hydration & Interactivity

## What is Hydration?

WebUI renders HTML at build time (or server-render time) with **zero JavaScript**. The browser displays the HTML immediately via [Declarative Shadow DOM](https://developer.chrome.com/docs/css-ui/declarative-shadow-dom) — users see content before any script loads.

**Hydration** is the process of attaching event listeners and reactive bindings to the already-rendered DOM. This is _not_ re-rendering: the DOM already exists. Hydration makes it interactive.

WebUI uses an **islands architecture**: only interactive components ship JavaScript. Static content stays static and never loads a framework.

```
Build time          Server render          Client hydration
─────────────       ───────────────        ─────────────────
Parse HTML    →     Render with state  →   Framework reconnects
with WebUI          + inject hydration     event handlers &
directives          markers                reactive bindings
```

## When to Hydrate

The default is **zero JavaScript**. You opt in to interactivity per-component.

- **Don't hydrate** pages that are purely informational — about pages, static content, read-only lists. These work with no JavaScript at all.
- **Hydrate** components that need: event handlers, reactive state updates, user input, or real-time data from the browser process.

If a page has ten components but only two need click handlers, only those two ship JavaScript.

## Two Hydration Paths

WebUI supports two hydration frameworks, chosen at build time:

```bash
webui build src --plugin=webui   # WebUI Framework
webui build src --plugin=fast    # FAST-HTML
```

### WebUI Framework (`--plugin=webui`)

The WebUI Framework provides automatic state seeding and targeted path-indexed updates. State is restored from SSR markers during the hydration walk — no manual DOM reading required.

```typescript
import { WebUIElement, attr, observable, volatile } from '@microsoft/webui-framework';

export class TodoApp extends WebUIElement {
  @attr title = '';
  @observable items: TodoItemData[] = [];

  @volatile get remainingCount(): number {
    return this.items.filter(i => i.state !== 'done').length;
  }

  addInput!: HTMLInputElement;

  onAddKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      const text = this.addInput.value.trim();
      if (!text) return;
      this.items = [...this.items, { id: String(Date.now()), title: text, state: 'pending' }];
      this.addInput.value = '';
    }
  }
}

TodoApp.define('todo-app');
```

Key characteristics:

- **Base class:** `WebUIElement` from `@microsoft/webui-framework`
- **Decorators:** `@attr`, `@observable`, `@volatile`
- **Refs:** `w-ref="name"`
- **State seeding:** automatic during hydration walk (no manual DOM reading)
- **Update model:** targeted path-indexed updates — only bindings referencing the changed property update
- **Registration:** `MyComponent.define('my-component')`

### FAST-HTML (`--plugin=fast`)

FAST-HTML builds on the [FAST](https://fast.design/) framework. State is restored manually in the `prepare()` method by reading the pre-rendered shadow DOM.

```typescript
import { FASTElement, attr, observable } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

export class TodoApp extends RenderableFASTElement(FASTElement) {
  @attr title = '';
  @observable items!: TodoItemData[];

  async prepare(): Promise<void> {
    const items: TodoItemData[] = [];
    for (const el of this.shadowRoot!.querySelectorAll('todo-item')) {
      items.push({
        id: el.getAttribute('id') || '',
        title: el.getAttribute('title') || '',
        state: el.getAttribute('state') || 'pending',
      });
    }
    this.items = items;
  }
}

TodoApp.defineAsync({ name: 'todo-app', templateOptions: 'defer-and-hydrate' });
```

Key characteristics:

- **Base class:** `RenderableFASTElement(FASTElement)` from `@microsoft/fast-html`
- **Decorators:** `@attr`, `@observable` (from `@microsoft/fast-element`)
- **Refs:** `f-ref="{name}"`
- **State seeding:** manual via `prepare()` method — read state from the pre-rendered DOM
- **Registration:** `MyComponent.defineAsync({ name: '...', templateOptions: 'defer-and-hydrate' })`

## Comparison

| Aspect | WebUI Framework | FAST-HTML |
|--------|----------------|-----------|
| Package | `@microsoft/webui-framework` | `@microsoft/fast-html` + `@microsoft/fast-element` |
| Base class | `WebUIElement` | `RenderableFASTElement(FASTElement)` |
| State seeding | Automatic from SSR markers | Manual in `prepare()` |
| Ref binding | `w-ref="name"` | `f-ref="{name}"` |
| Update model | Targeted path-indexed | Full observable chain |
| Best for | SSR-first, minimal JS | Complex client interactivity |

## Hydration Lifecycle

Both frameworks follow the same high-level lifecycle:

1. **Server renders HTML** with Declarative Shadow DOM
2. **Browser parses HTML** and creates shadow roots — content is visible immediately
3. **JavaScript loads** and custom elements upgrade
4. **Framework detects** the existing shadow root (instead of creating a new one)
5. **Walks DOM once** to connect bindings to SSR markers
6. **State is seeded** — automatically (WebUI Framework) or via `prepare()` (FAST-HTML)
7. **Markers are removed**, component is interactive

No flash of content — HTML is already visible from SSR, and hydration silently wires up the reactive state behind it.

## Performance Measurement

WebUI emits performance marks during hydration. Use them to verify that hydration is fast:

```typescript
window.addEventListener('webui:hydration-complete', () => {
  const total = performance.getEntriesByName('webui:hydrate:total', 'measure')[0];
  console.log(`Hydration complete in ${total?.duration.toFixed(1)}ms`);
});
```

## Template Syntax

Both frameworks use the same template syntax in component HTML files:

```html
<template shadowrootmode="open">
  <h1>{{title}}</h1>
  <button @click="{onClick()}">Click me</button>
  <for each="item in items">
    <p>{{item.name}}</p>
  </for>
  <if condition="isVisible">
    <span>Shown</span>
  </if>
</template>
```

Event binding, interpolation, conditionals, and loops work identically regardless of which framework you choose. The difference is in the TypeScript component class, not the template.

## Learn More

- [WebUI Framework examples](https://github.com/microsoft/webui/tree/main/examples/app/todo-webui)
- [FAST-HTML examples](https://github.com/microsoft/webui/tree/main/examples/app/todo-fast)
- [Plugins](/guide/concepts/plugins/) — How parser and handler plugins work
