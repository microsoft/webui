# Building a Todo App

This tutorial walks through the
[`todo-webui`](https://github.com/microsoft/webui/tree/main/examples/app/todo-webui)
example application. It uses the WebUI Framework to create two Web
Components - `<todo-app>` and `<todo-item>` - with reactive state, event
handling, and hydration from server-rendered HTML.

By the end you will know how to:

- Structure a WebUI project with components and static state
- Author templates that use WebUI directives (`<for>`, `<if>`, `{{}}`, `@click`, `w-ref`)
- Write TypeScript component classes with `@attr` and `@observable`
- Hydrate the page so the server-rendered markup becomes interactive

---

## 1. Project Setup

The example has the following layout:

```
todo-webui/
├── src/
│   ├── index.html
│   ├── index.ts
│   ├── todo-app/
│   │   ├── todo-app.html
│   │   ├── todo-app.css
│   │   └── todo-app.ts
│   └── todo-item/
│       ├── todo-item.html
│       ├── todo-item.css
│       └── todo-item.ts
└── data/
    └── state.json
```

- **src/** contains all source templates, styles, and client-side code.
- **data/** holds the JSON state that the WebUI server injects into the page at
  render time.

---

## 2. State

`data/state.json` provides the data for server-side rendering. The WebUI server
reads this file and uses it to populate every `{{expression}}` in your templates.

```json
{
  "textdirection": "ltr",
  "language": "en",
  "title": "Todo List",
  "remainingCount": 2,
  "items": [
    { "id": "1", "title": "Buy groceries", "state": "done" },
    { "id": "2", "title": "Write documentation", "state": "pending" },
    { "id": "3", "title": "Ship feature", "state": "pending" }
  ]
}
```

The `items` array drives the `<for>` loop inside the app component.
`remainingCount` is provided by the server so the initial render shows the
correct count. After hydration, the client keeps this value in sync by
updating the `@observable` in event handlers.

---

## 3. Entry Template

`src/index.html` is the outer HTML shell that the WebUI server renders first.

```html
<!DOCTYPE html>
<html lang="en" dir="{{textdirection}}">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>{{title}}</title>
</head>
<body>
  <todo-app title="{{title}}"></todo-app>
  <script type="module" src="/index.js"></script>
</body>
</html>
```

The `{{title}}` expressions are replaced with values from `state.json` at
render time. The module script at the bottom bootstraps hydration (see
[Section 7](#7-hydration-entry-point)).

---

## 4. Todo App Component

### Template – `src/todo-app/todo-app.html`

```html
<template shadowrootmode="open"
  @toggle-item="{onToggleItem(e)}"
  @delete-item="{onDeleteItem(e)}"
>
  <h1>{{title}}</h1>
  <div class="add-form">
    <input
      class="add-input"
      placeholder="What needs to be done?"
      w-ref="addInput"
      @keydown="{onAddKeydown(e)}"
    />
    <button class="add-button" @click="{onAddClick()}">Add</button>
  </div>
  <div class="todo-list">
    <for each="item in items">
      <todo-item
        id="{{item.id}}"
        title="{{item.title}}"
        state="{{item.state}}"
      ></todo-item>
    </for>
  </div>
  <div class="footer">
    <span>{{remainingCount}} items remaining</span>
  </div>
</template>
```

Key points:

- **`shadowrootmode="open"`** – the server emits a declarative shadow root so
  the component is visible before JavaScript loads.
- **`@toggle-item` / `@delete-item`** on the root `<template>` – these are
  delegated event listeners. Child `<todo-item>` elements emit these custom
  events, and the parent catches them here.
- **`w-ref="addInput"`** – stores a reference to the `<input>` element on the
  component class, accessible as `this.addInput`.
- **`@keydown` / `@click`** – WebUI event-binding syntax. The framework wires
  these to the corresponding methods on the component class.
- **`<for each="item in items">`** – iterates over the `items` array and stamps
  out a `<todo-item>` for each entry.

### Styles – `src/todo-app/todo-app.css`

```css
:host {
  display: block;
  max-width: 500px;
  margin: 0 auto;
  padding: 20px;
  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
}
.add-form {
  display: flex;
  gap: 8px;
  margin-bottom: 16px;
}
.add-input {
  flex: 1;
  padding: 8px;
  border: 1px solid #ccc;
  border-radius: 4px;
}
.add-button {
  padding: 8px 16px;
  background: #0078d4;
  color: #fff;
  border: none;
  border-radius: 4px;
  cursor: pointer;
}
.footer {
  margin-top: 16px;
  color: #666;
  font-size: 0.9em;
}
```

---

## 5. Todo Item Component

### Template – `src/todo-item/todo-item.html`

```html
<div class="todo-item" @click="{onClick(e)}">
  <button class="toggle" data-action="toggle" title="Toggle complete">
    <if condition="state == 'done'">
      <span class="check">&#10003;</span>
    </if>
  </button>
  <span class="title">{{title}}</span>
  <button class="delete" data-action="delete" title="Delete">&times;</button>
</div>
```

- **`<if condition="state == 'done'">`** – conditionally renders the checkmark
  only when the item is complete. This is evaluated during both server rendering
  and client-side reactive updates.
- **`data-action`** attributes – the component uses a single `@click` handler
  on the container and routes actions based on the `data-action` attribute of
  the clicked element.

### Styles – `src/todo-item/todo-item.css`

```css
:host {
  display: block;
}
.todo-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px;
  border-bottom: 1px solid #eee;
}
.todo-item .title {
  flex: 1;
}
:host([state="done"]) .title {
  text-decoration: line-through;
  color: #999;
}
.delete {
  background: none;
  border: none;
  color: #cc0000;
  font-size: 1.2em;
  cursor: pointer;
}
```

The `:host([state="done"])` selector applies strikethrough styling whenever the
host element's `state` attribute equals `"done"`.

---

## 6. Client-Side Component Classes

The TypeScript classes give each component its interactive behaviour. The WebUI
framework re-attaches these classes to the server-rendered shadow roots during
hydration.

### `src/todo-app/todo-app.ts`

```typescript
import { WebUIElement, attr, observable } from '@microsoft/webui-framework';

export class TodoApp extends WebUIElement {
  // Reflected attribute – kept in sync with the DOM attribute
  @attr title = '';

  // Observable array – changes trigger a re-render of the <for> loop
  @observable items: Array<{ id: string; title: string; state: string }> = [];

  // Remaining count – kept in sync by event handlers
  @observable remainingCount = 0;

  private updateRemaining(): void {
    this.remainingCount = (this.items ?? []).filter(i => i.state !== 'done').length;
  }

  // DOM reference populated by w-ref="addInput" in the template
  addInput!: HTMLInputElement;

  private nextId = 100;

  onAddKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      this.addTodo();
    }
  }

  onAddClick(): void {
    this.addTodo();
  }

  private addTodo(): void {
    const input = this.addInput;
    if (!input) return;

    const text = input.value.trim();
    if (!text) return;

    this.items = [
      ...this.items,
      { id: String(this.nextId++), title: text, state: 'pending' },
    ];
    input.value = '';
    input.focus();
  }

  onToggleItem(e: CustomEvent<{ id: string }>): void {
    const item = (this.items ?? []).find(i => i.id === e.detail.id);
    if (item) {
      item.state = item.state === 'done' ? 'pending' : 'done';
      this.items = [...this.items]; // Reassign to trigger reactive update
    }
  }

  onDeleteItem(e: CustomEvent<{ id: string }>): void {
    this.items = (this.items ?? []).filter(item => item.id !== e.detail.id);
  }
}

TodoApp.define('todo-app');
```

### `src/todo-item/todo-item.ts`

```typescript
import { WebUIElement, attr } from '@microsoft/webui-framework';

export class TodoItem extends WebUIElement {
  @attr id = '';
  @attr title = '';
  @attr state = 'pending';

  onClick(e: MouseEvent): void {
    const target = e.composedPath()[0] as HTMLElement;
    const action = target.closest('[data-action]')?.getAttribute('data-action');
    if (!action) return;

    if (action === 'toggle') {
      this.$emit('toggle-item', { id: this.id });
    } else if (action === 'delete') {
      this.$emit('delete-item', { id: this.id });
    }
  }
}

TodoItem.define('todo-item');
```

Note how `todo-item` uses `this.$emit()` to dispatch custom events that bubble
up to the parent `<todo-app>`, where they are caught by the `@toggle-item` and
`@delete-item` template bindings.

**Decorator summary:**

| Decorator | Purpose |
|-----------|---------|
| `@attr` | Reflects a class property to/from the element's HTML attribute. |
| `@observable` | Tracks changes and triggers template updates when the value is reassigned. |


---

## 7. Hydration Entry Point

`src/index.ts` imports the component modules so their custom element classes
are registered, which triggers the framework to walk the DOM and hydrate.

```typescript
window.addEventListener('webui:hydration-complete', logHydrationTiming);

function logHydrationTiming(): void {
  const total = performance.getEntriesByName('webui:hydrate:total', 'measure')[0];
  console.log(`Hydration complete in ${total?.duration.toFixed(1)}ms`);

}

// Side-effect imports - register custom elements and trigger hydration
import './todo-app/todo-app.js';
import './todo-item/todo-item.js';

// Fallback: if hydration already completed before the listener, log now
if (performance.getEntriesByName('webui:hydrate:total', 'measure').length > 0) {
  logHydrationTiming();
}
```

When the page loads:

1. The browser has already painted the server-rendered declarative shadow roots.
2. The module script runs, registering `todo-app` and `todo-item` as custom
   elements.
3. The framework matches each element to its class, re-attaches event listeners,
   and activates reactive bindings.
4. The `webui:hydration-complete` event fires once every component on the page
   has been hydrated. The timing breakdown shows how long each component took.

---

## 8. Build and Run

Install the WebUI toolchain:

```bash
npm install @microsoft/webui @microsoft/webui-framework
```

Start the development server with live reload:

```bash
npx webui serve ./src --state ./data/state.json --plugin=webui --watch
```

The `--state` flag tells the server which JSON file to use when rendering
templates. The `--watch` flag enables live reload on file changes.

Create a production build:

```bash
npx webui build ./src --out ./dist --plugin=webui
```

The output in `./dist` contains the compiled protocol binary and CSS files
ready for deployment with any handler (Rust, Node.js, C#, Python, Go).

---

## 9. What You've Learned

In this tutorial you:

- **Structured a WebUI project** with separate component directories for
  templates, styles, and TypeScript.
- **Created Web Components** using declarative shadow roots
  (`shadowrootmode="open"`) and WebUI template directives (`<for>`, `<if>`,
  `{{}}`).
- **Used `@attr` and `@observable`** decorators to manage reactive state in
  component classes.
- **Bound events** with `@click` and `@keydown` directives that map directly to
  class methods, and used `$emit()` for child-to-parent communication.
- **Referenced DOM elements** with `w-ref` to read input values without manual
  query selectors.
- **Hydrated the app** by importing component modules and listening for the
  `webui:hydration-complete` event with per-component timing.
- **Built and served** the app using the WebUI CLI.

---

## 10. Next Steps

- [Hydration Guide](/guide/concepts/hydration) – deep dive into how the
  framework re-attaches to server-rendered markup.
- [Routing](/guide/concepts/routing) – add multi-page navigation to your app.
- [Commerce Example](https://github.com/microsoft/webui/tree/main/examples/app/commerce) –
  a more complex app with product listings, search, cart, and nested routing.
- [FAST-HTML Variant](/guide/concepts/plugins/) – swap in FAST-HTML components
  using the `--plugin=fast` flag for an alternative hydration strategy.
