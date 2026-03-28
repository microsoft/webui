# Building a Todo App

This tutorial walks you through building a complete Todo application with the
WebUI Framework. You will create two Web Components—`<todo-app>` and
`<todo-item>`—wire up reactive state with decorators, handle user events, and
hydrate the app from server-rendered HTML.

By the end you will know how to:

- Structure a WebUI project with components and static state
- Author templates that use WebUI directives (`<for>`, `{{}}`, `@click`, `w-ref`)
- Write TypeScript component classes with `@attr`, `@observable`, and `@volatile`
- Hydrate the page so the server-rendered markup becomes interactive

---

## 1. Project Setup

Create the following directory layout:

```
todo-app/
├── src/
│   ├── index.html
│   ├── index.ts
│   ├── todo-app/
│   │   ├── todo-app.html
│   │   └── todo-app.css
│   └── todo-item/
│       ├── todo-item.html
│       └── todo-item.css
└── data/
    └── state.json
```

- **src/** contains all source templates, styles, and client-side code.
- **data/** holds the JSON state that the WebUI server injects into the page at
  render time.

---

## 2. State

Create `data/state.json`. The WebUI server reads this file and uses it to
populate every `{{expression}}` in your templates during server-side rendering.

```json
{
  "title": "My Todos",
  "items": [
    { "id": "1", "title": "Learn WebUI", "state": "done" },
    { "id": "2", "title": "Build a todo app", "state": "pending" },
    { "id": "3", "title": "Ship to production", "state": "pending" }
  ]
}
```

The `items` array drives the `<for>` loop inside the app component, and `title`
is interpolated into both the page `<title>` and the `<h1>`.

---

## 3. Entry Template

Create `src/index.html`. This is the outer HTML shell that the WebUI server
renders first.

```html
<!DOCTYPE html>
<html lang="en">
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

The `{{title}}` expressions are replaced with the value from `state.json` at
render time. The module script at the bottom bootstraps hydration (see
[Section 7](#7-hydration-entry-point)).

---

## 4. Todo App Component

### Template – `src/todo-app/todo-app.html`

```html
<template shadowrootmode="open">
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
- **`w-ref="addInput"`** – creates a typed reference to the `<input>` element
  on the component class, accessible as `this.addInput`.
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
<template shadowrootmode="open">
  <div class="todo-item">
    <input type="checkbox" ?checked="{{checked}}" @change="{onToggle()}" />
    <span class="title">{{title}}</span>
    <button class="delete" @click="{onDelete()}">×</button>
  </div>
</template>
```

- **`?checked="{{checked}}"`** – a boolean attribute binding. When `checked`
  evaluates to `true`, the `checked` attribute is set; otherwise it is removed.
- **`@change` / `@click`** – event bindings that call methods on the component.

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

Create the TypeScript classes that give each component its interactive
behaviour. The WebUI framework re-attaches these classes to the server-rendered
shadow roots during hydration.

### `src/todo-app/todo-app.ts`

```typescript
import { WebUIElement, attr, observable, volatile, ref } from '@microsoft/webui-framework';

export class TodoApp extends WebUIElement {
  // Reflected attribute – kept in sync with the DOM attribute
  @attr title: string = '';

  // Observable array – changes trigger a re-render of the <for> loop
  @observable items: Array<{ id: string; title: string; state: string }> = [];

  // Volatile computed – recalculated on every access, never cached
  @volatile get remainingCount(): number {
    return this.items.filter(i => i.state === 'pending').length;
  }

  // Typed DOM reference created by w-ref="addInput" in the template
  @ref addInput!: HTMLInputElement;

  onAddKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      this.addItem();
    }
  }

  onAddClick(): void {
    this.addItem();
  }

  private addItem(): void {
    const value = this.addInput.value.trim();
    if (!value) return;

    this.items = [
      ...this.items,
      { id: crypto.randomUUID(), title: value, state: 'pending' },
    ];
    this.addInput.value = '';
    this.addInput.focus();
  }

  connectedCallback(): void {
    super.connectedCallback();

    this.addEventListener('todo-toggle', ((e: CustomEvent) => {
      this.items = this.items.map(i =>
        i.id === e.detail.id
          ? { ...i, state: i.state === 'done' ? 'pending' : 'done' }
          : i,
      );
    }) as EventListener);

    this.addEventListener('todo-delete', ((e: CustomEvent) => {
      this.items = this.items.filter(i => i.id !== e.detail.id);
    }) as EventListener);
  }
}

TodoApp.define('todo-app');
```

### `src/todo-item/todo-item.ts`

```typescript
import { WebUIElement, attr } from '@microsoft/webui-framework';

export class TodoItem extends WebUIElement {
  @attr id: string = '';
  @attr title: string = '';
  @attr state: string = 'pending';

  get checked(): boolean {
    return this.state === 'done';
  }

  onToggle(): void {
    this.dispatchEvent(
      new CustomEvent('todo-toggle', {
        bubbles: true,
        composed: true,
        detail: { id: this.id },
      }),
    );
  }

  onDelete(): void {
    this.dispatchEvent(
      new CustomEvent('todo-delete', {
        bubbles: true,
        composed: true,
        detail: { id: this.id },
      }),
    );
  }
}

TodoItem.define('todo-item');
```

**Decorator summary:**

| Decorator | Purpose |
|-----------|---------|
| `@attr` | Reflects a class property to/from the element's HTML attribute. |
| `@observable` | Tracks changes and triggers template updates when the value is reassigned. |
| `@volatile` | Marks a getter as non-cacheable—it is re-evaluated every time the template reads it. |
| `@ref` | Binds a property to the DOM element marked with the matching `w-ref` attribute. |

---

## 7. Hydration Entry Point

Create `src/index.ts`. This module imports every component so their classes are
registered before the framework walks the DOM to hydrate.

```typescript
import './todo-app/todo-app.js';
import './todo-item/todo-item.js';

window.addEventListener('webui:hydration-complete', () => {
  console.log('Todo app is interactive!');
});
```

When the page loads:

1. The browser has already painted the server-rendered declarative shadow roots.
2. The module script runs, registering `todo-app` and `todo-item` as custom
   elements.
3. The framework matches each element to its class, re-attaches event listeners,
   and activates reactive bindings.
4. The `webui:hydration-complete` event fires once every component on the page
   has been hydrated.

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

The output in `./dist` contains pre-rendered HTML with declarative shadow roots
and bundled JavaScript ready for deployment.

---

## 9. What You've Learned

In this tutorial you:

- **Structured a WebUI project** with separate component directories for
  templates, styles, and TypeScript.
- **Created Web Components** using declarative shadow roots
  (`shadowrootmode="open"`) and WebUI template directives (`<for>`, `{{}}`).
- **Used `@attr`, `@observable`, and `@volatile`** decorators to manage reactive
  state inside component classes.
- **Bound events** with `@click`, `@keydown`, and `@change` directives that map
  directly to class methods.
- **Referenced DOM elements** with `w-ref` to read input values without manual
  query selectors.
- **Hydrated the app** by importing component modules and listening for the
  `webui:hydration-complete` event.
- **Built and served** the app using the WebUI CLI.

---

## 10. Next Steps

- [Hydration Guide](/guide/concepts/hydration/) – deep dive into how the
  framework re-attaches to server-rendered markup.
- [Routing](/guide/concepts/routing/) – add multi-page navigation to your app.
- [Commerce Example](/examples/commerce/) – a more complex app with cart state,
  product listings, and checkout.
- [FAST-HTML Variant](/guide/concepts/plugins/) – swap in FAST-HTML components
  using the `--plugin=fast` flag for an alternative hydration strategy.
