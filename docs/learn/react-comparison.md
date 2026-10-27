---
description: "Translate React skills into a new WebUI app: components, initial state, props, events, keys, CSS, and composition."
---

<span id="react-vs-web-components"></span>

# Coming from React

You already know the useful ideas: describe UI from state, compose components,
pass data down, and handle user intent. WebUI keeps those ideas, but puts
structure in HTML templates, styling in CSS, and browser behavior in
TypeScript.

Both systems are declarative. React supports client rendering, SSR, and
[Server Components](https://react.dev/reference/rsc/server-components).
Server Component source does not ship to the browser, and React SSR can show
content before hydration. The relevant difference is how a particular
application runs, not whether one framework is "declarative" or "fast."

This guide is for **starting a new WebUI app**. It is not a JSX drop-in:
React components, hooks, context providers, and React-specific UI libraries
need a different implementation. Framework-independent JavaScript and ordinary
web APIs may still be useful.

## Key Differences at a Glance

| Familiar concept | In WebUI |
| --- | --- |
| Component | A hyphenated HTML template with paired CSS and optional TypeScript |
| JSX expressions | Path bindings, `<if>`, and `<for>`; no arbitrary JavaScript in templates |
| Initial render | Your backend provides JSON; the native renderer does not run component TypeScript |
| Client state | `@observable` properties and assignments after hydration |
| Props | HTML attributes and `@attr`; complex values use property bindings such as `:items` |
| Callback props | Native events and bubbling, composed custom events |
| List identity | A stable `key` on the first concrete repeated child |
| CSS | Global in Light DOM; natively isolated in Shadow DOM |
| `children` | Explicit nesting and, in Shadow components, native slots |

Follow [Add interaction](/start/add-interactivity) for the counter's
complete files and build commands. If you have not created a project yet,
start with [Your first page](/start/first-page). The counter below uses
the lesson's files; other examples are focused snippets.

## Simple Counter

Both examples display an initial count and increment it on click.

<code-comparison left-label="React" right-label="WebUI">
<div slot="left">

```jsx
'use client';

import { useState } from 'react';

export function Counter({ label = 'Clicks', initialCount = 0 }) {
  const [count, setCount] = useState(initialCount);

  return (
    <div>
      <p>{label}: {count}</p>
      <button type="button" onClick={() => setCount(value => value + 1)}>
        Increment
      </button>
    </div>
  );
}
```

`'use client'` marks the client boundary when using a React Server Components
framework. It is not needed in every React setup.

</div>
<div slot="right">

**`src/my-counter/my-counter.html`**

```html
<p>{{label}}: {{count}}</p>
<button type="button" @click="{increment()}">Increment</button>
```

**`src/my-counter/my-counter.ts`**

```typescript
// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr, observable } from '@microsoft/webui-framework';

export class MyCounter extends WebUIElement {
  @attr label = '';
  @observable count = 0;

  increment(): void {
    this.count += 1;
  }
}

MyCounter.define('my-counter');
```

</div>
</code-comparison>

In the page template, place `<my-counter label="Clicks"></my-counter>`.
Supply `{ "count": 0 }` in server state, import the component from the browser
entry, and load the bundled entry with a module script.

**The initial-state difference matters:** a WebUI field initializer is not
executed by the renderer. The server must provide `count` for the first HTML;
the component handles changes after hydration. No server-side execution of
hooks, constructors, or computed getters fills that gap.

## Conditional Rendering

<code-comparison left-label="React" right-label="WebUI">
<div slot="left">

```jsx
function Greeting({ isLoggedIn, username }) {
  return isLoggedIn
    ? <p>Welcome back, {username}!</p>
    : <p>Please sign in.</p>;
}
```

</div>
<div slot="right">

```html
<if condition="isLoggedIn">
  <p>Welcome back, {{username}}!</p>
</if>
<if condition="!isLoggedIn">
  <p>Please sign in.</p>
</if>
```

</div>
</code-comparison>

WebUI can render this template from `{ "isLoggedIn": true, "username": "Ada" }`
without a component class. If browser behavior later changes those values,
add the appropriate observable state and runtime.

Text bindings are **path lookups**, not JavaScript expressions.
<code v-pre>{{items.length}}</code> works;
<code v-pre>{{items.map(...)}}</code>, <code v-pre>{{count + 1}}</code>, and
<code v-pre>{{formatDate(date)}}</code> do not. Precompute formatted initial values on the
server. Conditions allow comparisons and limited logical operators, not
ternaries, function calls, arithmetic, grouping parentheses, or mixed `&&`
and `||`. See the [directive reference](/learn/directives/).

## List Rendering

Both versions render an array of `{ id, title }` records:

<code-comparison left-label="React" right-label="WebUI">
<div slot="left">

```jsx
function TodoList({ items }) {
  return (
    <ul>
      {items.map(item => <li key={item.id}>{item.title}</li>)}
    </ul>
  );
}
```

</div>
<div slot="right">

```html
<ul>
  <for each="item in items">
    <li key="{{item.id}}">{{item.title}}</li>
  </for>
</ul>
```

</div>
</code-comparison>

WebUI renders the initial array on the server. With browser state updates,
stable keys preserve repeated content across reorder or removal. Omit keys
only when positional identity is appropriate. Put the key on the first
concrete child, not on `<for>`; `id` and `data-key` are not implicit keys.

A component inside the loop receives explicit attributes or properties:

```html
<for each="item in items">
  <todo-item key="{{item.id}}" id="{{item.id}}" title="{{item.title}}"></todo-item>
</for>
```

Its own template uses `title`, not the parent's `item.title`.

## Event Handling

React uses event props; WebUI binds native DOM events. Both examples keep an
input and its displayed value in sync:

<code-comparison left-label="React" right-label="WebUI">
<div slot="left">

```jsx
import { useState } from 'react';

function SearchBox() {
  const [query, setQuery] = useState('');
  return (
    <label>
      Filter
      <input value={query} onChange={e => setQuery(e.target.value)} />
      <span>{query}</span>
    </label>
  );
}
```

</div>
<div slot="right">

```html
<label>
  Filter
  <input value="{{query}}" @input="{onInput(e)}">
  <span>{{query}}</span>
</label>
```

Inside the component class:

```typescript
@observable query = '';

onInput(e: Event): void {
  this.query = (e.currentTarget as HTMLInputElement).value;
}
```

</div>
</code-comparison>

Supply `query` in WebUI's server state too. Event arguments can be `e`, paths,
or supported literals; do calculations in the handler, not in `@input`.
`w-ref="{inputEl}"` is for imperative APIs such as focus, not reading rendered
text back as application state.

## Parent-Child Communication

**Data down:** use attributes for scalar inputs and property bindings for
complex values. An authored component can expose them as:

```typescript
@attr label = '';
@attr({ mode: 'boolean' }) disabled = false;
@observable items: Array<{ id: string; title: string }> = [];
```

```html
<task-list label="Today" ?disabled="{{isSaving}}" :items="{{items}}"></task-list>
```

Boolean attributes use presence/absence; `disabled="false"` is still present.
Property bindings use the `:` prefix on custom elements, not native inputs.

**Intent up:** a React child might call `onDelete(id)`. A WebUI child can emit:

```typescript
this.$emit('delete-item', { id: this.id });
```

The parent template listens on the child:

```html
<todo-item id="1" @delete-item="{onDeleteItem(e)}"></todo-item>
```

The parent class owns the resulting state change:

```typescript
onDeleteItem(e: CustomEvent<{ id: string }>): void {
  this.items = this.items.filter(item => item.id !== e.detail.id);
}
```

`$emit()` sets `bubbles` and `composed`, so events can cross Shadow DOM
boundaries. Native `submit` and `change` events do not do that; bind them on
the form or input itself. The [Todo tutorial](/start/todo-app) combines
these patterns in a runnable app.

## Styling

React can use ordinary CSS; CSS-in-JS is optional. WebUI uses ordinary paired
CSS files rather than JavaScript-driven styling.

- **Shadow is the default** for an unwrapped WebUI component. Use `:host` for
  the host, and ordinary selectors for its internal elements.
- **Light DOM is global** in its containing Document or ShadowRoot. Selectors
  are not rewritten or scoped. A rule such as `.title` may affect other Light
  components in that tree.
- A sole bare `<template>` explicitly selects Light DOM. A sole
  `<template shadowrootmode="open">` selects Shadow even in a `--dom light`
  build. `<slot>` and `:host` require Shadow.

Let state control an attribute, then let CSS control appearance:

```html
<button aria-pressed="{{isActive}}" ?data-active="{{isActive}}" @click="{toggle()}">
  Favorite
</button>
```

```css
button[data-active] {
  font-weight: 700;
}
```

`aria-pressed` takes a value including `"false"`; it is not an HTML boolean
attribute. The `?` prefix is appropriate for the presence-based styling hook.

## Component Composition

React's `children` prop and native slots solve similar composition tasks, but
they are not interchangeable APIs.

<code-comparison left-label="React" right-label="WebUI">
<div slot="left">

```jsx
function NoticePanel({ heading, children }) {
  return <section><h2>{heading}</h2>{children}</section>;
}

function App() {
  return (
    <NoticePanel heading="Next">
      <a href="/tasks">Open tasks</a>
    </NoticePanel>
  );
}
```

</div>
<div slot="right">

Component template:

```html
<template shadowrootmode="open">
  <section>
    <h2>{{heading}}</h2>
    <slot></slot>
  </section>
</template>
```

Parent template:

```html
<notice-panel heading="Next">
  <a href="/tasks">Open tasks</a>
</notice-panel>
```

</div>
</code-comparison>

Use slots for caller-provided content in Shadow components and `<outlet>` for
child routes. Light components do not support native slots. Keep structure in
templates instead of building it with DOM APIs.

**Next:** [build the counter](/start/add-interactivity), or use the
[best-practices checklist](/reference/best-practices) while authoring.
