---
description: Build and bundle a WebUI todo app with server state, keyed lists, native events, and observable updates.
---

# Building a Todo App

Build two components: `<todo-app>` owns a list, and `<todo-item>` reports
actions through custom events. The initial list is server-rendered; adding,
toggling, and deleting items happen in the browser.

Complete [Hello World](/tutorials/hello-world) first. This tutorial uses the
same two-component structure as the repository's
[`todo-webui` example](https://github.com/microsoft/webui/tree/main/examples/app/todo-webui),
with a labeled native form and a standalone npm setup. It does not save changes
to a database: reloading restores `data/state.json`.

## 1. Project Setup

Create a separate project. Run all commands from its root:

```bash
mkdir todo-webui
cd todo-webui
```

**`package.json`**

```json
{
  "name": "todo-webui",
  "private": true,
  "type": "module",
  "scripts": {
    "build:client": "node build-client.mjs",
    "build:protocol": "webui build ./src --out ./dist --plugin webui",
    "build": "npm run build:client && npm run build:protocol",
    "dev": "webui serve ./src --state ./data/state.json --plugin webui --servedir ./dist"
  }
}
```

```bash
npm install @microsoft/webui-framework
npm install --save-dev @microsoft/webui esbuild
```

Keep WebUI packages on matching releases and keep optional dependencies enabled.
Create the files below in your editor:

```text
todo-webui/
  package.json
  tsconfig.json
  build-client.mjs
  data/state.json
  src/
    index.html
    index.ts
    todo-app/
      todo-app.html
      todo-app.css
      todo-app.ts
    todo-item/
      todo-item.html
      todo-item.css
      todo-item.ts
```

## 2. State

**`data/state.json`**

```json
{
  "title": "Todo List",
  "draft": "",
  "remainingCount": 2,
  "items": [
    { "id": "1", "title": "Buy groceries", "state": "done" },
    { "id": "2", "title": "Write documentation", "state": "pending" },
    { "id": "3", "title": "Ship feature", "state": "pending" }
  ]
}
```

`remainingCount` is computed data: the server supplies its initial value, and
each client mutation updates it. The renderer does not execute a TypeScript
getter or constructor to calculate it.

## 3. Entry Template

**`src/index.html`**

```html
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>{{title}}</title>
</head>
<body>
  <main>
    <todo-app title="{{title}}"></todo-app>
  </main>
  <script type="module" src="/index.js"></script>
</body>
</html>
```

The script URL points to the bundle created in
[Build and Run](#8-build-and-run), not directly to a TypeScript file.

## 4. Todo App Component

### Template – `src/todo-app/todo-app.html`

```html
<template shadowrootmode="open"
  @toggle-item="{onToggleItem(e)}"
  @delete-item="{onDeleteItem(e)}"
>
  <h1>{{title}}</h1>
  <form @submit="{onAdd(e)}">
    <label for="new-task">New task</label>
    <div class="add-form">
      <input
        id="new-task"
        type="text"
        required
        value="{{draft}}"
        @input="{onInput(e)}"
        w-ref="{addInput}"
      >
      <button type="submit">Add</button>
    </div>
  </form>
  <div class="todo-list">
    <for each="item in items">
      <todo-item
        key="{{item.id}}"
        id="{{item.id}}"
        title="{{item.title}}"
        state="{{item.state}}"
      ></todo-item>
    </for>
  </div>
  <p role="status">Remaining tasks: {{remainingCount}}</p>
</template>
```

- The sole open `<template>` selects Shadow DOM. Its root event bindings
  listen on the host for child custom events.
- `@submit` belongs on the form: native submit events do not cross a shadow
  boundary. Enter and the Add button use the same handler.
- `w-ref="{addInput}"` needs braces. The ref is used only to restore focus;
  `@input` updates `draft`, and the `value` binding keeps the input in sync.
- <code v-pre>key="{{item.id}}"</code> preserves each repeated component's identity. `id` alone
  is not a repeat key.
- Child components receive item values through attributes; their templates
  do not inherit the parent's `item` variable.

### Styles – `src/todo-app/todo-app.css`

```css
:host {
  display: block;
  max-inline-size: 32rem;
  margin-inline: auto;
  padding: 1rem;
  font-family: system-ui, sans-serif;
}

.add-form {
  display: flex;
  gap: 0.5rem;
  margin-block: 0.5rem 1rem;
}

input {
  flex: 1;
  min-inline-size: 0;
}

input, button {
  font: inherit;
  padding: 0.5rem;
}
```

## 5. Todo Item Component

### Template – `src/todo-item/todo-item.html`

```html
<div class="todo-item">
  <button type="button" @click="{toggle()}">Toggle complete</button>
  <span class="title">{{title}}</span>
  <if condition="state == 'done'">
    <span>Done</span>
  </if>
  <button type="button" @click="{deleteItem()}">Delete</button>
</div>
```

The unwrapped template also uses Shadow DOM by default. Each button binds its
own native click handler. The visible "Done" label makes completion
understandable without relying only on color or strikethrough.

### Styles – `src/todo-item/todo-item.css`

```css
:host {
  display: block;
}

.todo-item {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 0.5rem;
  padding-block: 0.5rem;
  border-block-end: 1px solid currentColor;
}

.title {
  flex: 1;
  overflow-wrap: anywhere;
}

:host([state="done"]) .title {
  text-decoration: line-through;
}

button {
  font: inherit;
  padding: 0.5rem;
}
```

## 6. Client-Side Component Classes

### `src/todo-app/todo-app.ts`

```typescript
// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr, observable } from '@microsoft/webui-framework';

interface Todo {
  id: string;
  title: string;
  state: string;
}

export class TodoApp extends WebUIElement {
  @attr title = '';
  @observable items: Todo[] = [];
  @observable draft = '';
  @observable remainingCount = 0;

  addInput!: HTMLInputElement;

  onInput(e: Event): void {
    this.draft = (e.currentTarget as HTMLInputElement).value;
  }

  onAdd(e: SubmitEvent): void {
    e.preventDefault();
    const title = this.draft.trim();
    if (!title) return;

    this.items = [
      ...this.items,
      { id: crypto.randomUUID(), title, state: 'pending' },
    ];
    this.updateRemaining();
    this.draft = '';
    this.addInput.focus();
  }

  onToggleItem(e: CustomEvent<{ id: string }>): void {
    const item = this.items.find(item => item.id === e.detail.id);
    if (!item) return;
    item.state = item.state === 'done' ? 'pending' : 'done';
    this.items = [...this.items];
    this.updateRemaining();
  }

  onDeleteItem(e: CustomEvent<{ id: string }>): void {
    this.items = this.items.filter(item => item.id !== e.detail.id);
    this.updateRemaining();
  }

  private updateRemaining(): void {
    this.remainingCount = this.items.filter(item => item.state !== 'done').length;
  }
}

TodoApp.define('todo-app');
```

Reassigning `items` notifies its observable setter; changing a nested field
alone does not. The count is updated after **every** add, toggle, and delete.
`crypto.randomUUID()` avoids collisions with initial IDs and is available on
localhost or HTTPS.

### `src/todo-item/todo-item.ts`

```typescript
// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '@microsoft/webui-framework';

export class TodoItem extends WebUIElement {
  @attr id = '';
  @attr title = '';
  @attr state = '';

  toggle(): void {
    this.$emit('toggle-item', { id: this.id });
  }

  deleteItem(): void {
    this.$emit('delete-item', { id: this.id });
  }
}

TodoItem.define('todo-item');
```

The child reports intent rather than mutating the parent's list. `$emit()`
creates a bubbling, composed custom event, so it can reach the parent host
across Shadow DOM boundaries.

## 7. Hydration Entry Point

**`src/index.ts`**

```typescript
// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import './todo-app/todo-app.js';
import './todo-item/todo-item.js';
```

These imports register the elements. The runtime adopts the rendered DOM and
connects behavior; no explicit "hydrate the page" call is needed. The initial
content remains visible without JavaScript, but the todo actions need this
bundle.

## 8. Build and Run

**`tsconfig.json`**

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "experimentalDecorators": true,
    "useDefineForClassFields": false
  }
}
```

**`build-client.mjs`**

```javascript
// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { build } from 'esbuild';

await build({
  entryPoints: ['src/index.ts'],
  outfile: 'dist/index.js',
  bundle: true,
  format: 'esm',
  target: 'es2022',
  tsconfig: 'tsconfig.json',
  define: { __WEBUI_DEV__: 'true' },
});
```

```bash
npm run build
npm run dev
```

Open **http://127.0.0.1:3000/**. You should see three tasks and **Remaining
tasks: 2**. Toggle "Write documentation" to see **Remaining tasks: 1**. Add a
task to return to **2**, then delete that task to return to **1**.

The two build steps have different outputs:

| Step | Output |
| --- | --- |
| `build:client` | `dist/index.js`, including the runtime and your component classes |
| `build:protocol` | `dist/protocol.bin` and generated component CSS |

The preview server renders the templates and serves the bundle through
`--servedir ./dist`. `webui build` alone does not produce `index.js`.
If the page is visible but buttons do nothing, check that the bundle was built
and that `/index.js` returns JavaScript.

Add `--watch` to the dev command for template/state reloads. Rebuild browser code
with `npm run build:client` after TypeScript edits, then refresh. This setup
transforms TypeScript but does not type-check it. For release bundles, set
`__WEBUI_DEV__` to `'false'` and enable esbuild's `minify` option after testing.

## 9. What You've Learned

- Server state produces the first list and count.
- Component attributes carry data down; custom events report actions up.
- Observable assignments update bindings, and repeat keys preserve identity.
- The form uses normal submit/input events; the ref is only for focus.
- Templates and browser TypeScript have separate build steps.

## 10. Next Steps

Use your [backend integration](/guide/integrations) to persist changes, handle
failed writes, and return fresh initial state. Then explore
[hydration](/guide/concepts/hydration) or [routing](/guide/concepts/routing)
when the application needs them.
