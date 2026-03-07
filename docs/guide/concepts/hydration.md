# Hydration & Interactivity

WebUI renders HTML at build time with zero JavaScript. To add client-side interactivity — event handlers, reactive state, dynamic updates — you use [FAST](https://fast.design/) custom elements that **hydrate** the pre-rendered HTML on the client. This is an islands architecture: most of the page is static HTML, and only interactive components ship JavaScript.

```
Build time          Server render          Client hydration
─────────────       ───────────────        ─────────────────
Parse HTML    →     Render with state  →   FAST reconnects
with WebUI          + inject hydration     event handlers &
directives          markers                reactive bindings
```

## Class Definition

Define a custom element by extending `RenderableFASTElement(FASTElement)` and registering it with `defineAsync`:

```typescript
import { FASTElement, attr, observable } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

export class MyCounter extends RenderableFASTElement(FASTElement) {
  @attr count = 0;
  @observable label!: string;

  onIncrement(): void {
    this.count++;
  }
}

MyCounter.defineAsync({
  name: 'my-counter',
  templateOptions: 'defer-and-hydrate',
});
```

## Templating

Each component has an HTML template with `shadowrootmode="open"`. Use `{{expr}}` for bindings:

```html
<template shadowrootmode="open">
  <p>{{label}}: {{count}}</p>
  <button @click="{onIncrement()}">+1</button>
</template>
```

## Observation

| Decorator | Purpose | Example |
|-----------|---------|---------|
| `@attr` | Two-way binding with HTML attribute | `@attr title = ''` |
| `@observable` | Reactive property — triggers re-render on change | `@observable items!: Item[]` |

```typescript
export class TodoItem extends RenderableFASTElement(FASTElement) {
  @attr title = '';           // <todo-item title="Buy milk">
  @attr state = 'pending';   // <todo-item state="done">
  @observable editing = false; // Internal reactive state
}
```

## Events

Bind events with `@eventname="{handler()}"`:

| Syntax | Behavior |
|--------|----------|
| `@click="{onClick()}"` | Call handler, no event object |
| `@click="{onClick(e)}"` | Call handler with the event object |
| `@keydown="{onKey(e)}"` | Works with any DOM event |
| `@custom-event="{onCustom(e)}"` | Works with `CustomEvent` (access `e.detail`) |

```html
<button @click="{onSave()}">Save</button>
<input @keydown="{onKeydown(e)}" />
<todo-item @toggle="{onToggle(e)}"></todo-item>
```

## References

Use `f-ref` to store a DOM element reference on the component instance:

```html
<input f-ref="{nameInput}" @keydown="{onKeydown(e)}" />
```

```typescript
export class MyForm extends RenderableFASTElement(FASTElement) {
  nameInput!: HTMLInputElement;  // Populated by f-ref

  onKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      console.log(this.nameInput.value);
    }
  }
}
```

## Initial State

`prepare()` is called **after hydration** — the DOM is already rendered, and FAST has reconnected bindings. Use it to restore component state from the pre-rendered shadow DOM:

```typescript
async prepare(): Promise<void> {
  const items: Item[] = [];
  for (const el of this.shadowRoot!.querySelectorAll('todo-item')) {
    items.push({
      id: el.getAttribute('id') || '',
      title: el.getAttribute('title') || '',
      state: el.getAttribute('state') || 'pending',
    });
  }
  this.items = items;  // Setting @observable triggers reactivity
}
```

No flash of content — HTML is already visible from SSR, and `prepare()` silently wires up the reactive state behind it.

## Full Example

See the [`todo-fast`](https://github.com/microsoft/webui/tree/main/examples/app/todo-fast) example for a complete app.

```bash
webui build examples/app/todo-fast/src --out dist --plugin=fast

webui serve examples/app/todo-fast/src \
  --state examples/app/todo-fast/data/state.json \
  --plugin=fast --watch
```

## Learn More

- [FAST documentation](https://fast.design/) — Full framework reference
- [Plugins](/guide/concepts/plugins/) — How the `fast` build plugin works internally
