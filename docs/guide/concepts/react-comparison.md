# React vs Web Components

This guide compares common UI patterns written in React (imperative, JavaScript-centric) with their WebUI and FAST equivalents (declarative, HTML-centric). Each section shows React and WebUI side by side so you can see how familiar patterns translate to a Web Components model.

## Key Differences at a Glance

| | React | WebUI |
|---|---|---|
| **Component model** | JSX functions / classes with virtual DOM | Web Components with Shadow DOM |
| **Rendering** | Client-side or Node.js SSR | Build-time compiled protocol, server-rendered HTML |
| **Template language** | JSX (JavaScript + HTML mixed) | Separate HTML, CSS, and TypeScript files |
| **State management** | `useState`, `useReducer`, context | `@observable` properties with targeted DOM updates |
| **Styling** | CSS-in-JS, CSS Modules, or external | Scoped CSS via Shadow DOM |
| **Runtime** | React runtime + ReactDOM in browser | No framework runtime for static content; thin hydration for interactive islands |
| **Interactivity** | Every component ships JavaScript | Only interactive islands ship JavaScript |

## Simple Counter

<CodeComparison>
<template #left>

```jsx
import { useState } from 'react';

function Counter() {
  const [count, setCount] = useState(0);

  return (
    <div>
      <p>Count: {count}</p>
      <button onClick={() => setCount(count + 1)}>Increment</button>
    </div>
  );
}
```

</template>
<template #right>

**my-counter.html**

```html
<p>Count: {{count}}</p>
<button @click="{increment()}">Increment</button>
```

**my-counter.ts**

```typescript
import { WebUIElement, observable } from '@microsoft/webui-framework';

export class MyCounter extends WebUIElement {
  @observable count = 0;

  increment(): void {
    this.count += 1;
  }
}

MyCounter.define('my-counter');
```

</template>
</CodeComparison>

**What changed:** Template and logic are separated into HTML and TypeScript files. No JSX, no `useState` hook, no `setState` call. The `@observable` decorator makes `count` reactive - when it changes, only the bound DOM nodes update.

## Conditional Rendering

<CodeComparison>
<template #left>

```jsx
function Greeting({ isLoggedIn, username }) {
  return (
    <div>
      {isLoggedIn ? (
        <p>Welcome back, {username}!</p>
      ) : (
        <p>Please sign in.</p>
      )}
    </div>
  );
}
```

</template>
<template #right>

**user-greeting.html**

```html
<div>
  <if condition="isLoggedIn">
    <p>Welcome back, {{username}}!</p>
  </if>
  <if condition="!isLoggedIn">
    <p>Please sign in.</p>
  </if>
</div>
```

</template>
</CodeComparison>

**What changed:** Conditional logic moves from JavaScript ternary expressions into declarative `<if>` directives. These are evaluated on the server during rendering - no JavaScript is shipped to the browser for static conditionals.

## List Rendering

<CodeComparison>
<template #left>

```jsx
function TodoList({ items }) {
  return (
    <ul>
      {items.map((item) => (
        <li key={item.id}>
          <span>{item.title}</span>
          <span className={`status ${item.state}`}>{item.state}</span>
        </li>
      ))}
    </ul>
  );
}
```

</template>
<template #right>

**todo-list.html**

```html
<ul>
  <for each="item in items">
    <li>
      <span>{{item.title}}</span>
      <span class="status {{item.state}}">{{item.state}}</span>
    </li>
  </for>
</ul>
```

</template>
</CodeComparison>

**What changed:** `Array.map()` with JSX becomes a declarative `<for>` directive. The `key` prop is replaced by the first attribute on the repeated element. This runs on the server and produces static HTML - no JavaScript array iteration in the browser.

## Event Handling

<CodeComparison>
<template #left>

```jsx
function SearchBox() {
  const [query, setQuery] = useState('');

  const performSearch = (searchQuery) => {
    // search logic
  };

  const handleKeyDown = (e) => {
    if (e.key === 'Enter') {
      performSearch(query);
    }
  };

  return (
    <div>
      <input
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        onKeyDown={handleKeyDown}
        placeholder="Search..."
      />
      <button onClick={() => performSearch(query)}>Search</button>
    </div>
  );
}
```

</template>
<template #right>

**search-box.html**

```html
<div>
  <input
    @input="{onInput(e)}"
    @keydown="{onKeyDown(e)}"
    placeholder="Search..."
  />
  <button @click="{performSearch()}">Search</button>
</div>
```

**search-box.ts**

```typescript
import { WebUIElement, observable } from '@microsoft/webui-framework';

export class SearchBox extends WebUIElement {
  @observable query = '';

  onInput(e: InputEvent): void {
    this.query = (e.currentTarget as HTMLInputElement).value;
  }

  onKeyDown(e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      this.performSearch();
    }
  }

  performSearch(): void {
    // search logic using this.query
  }
}

SearchBox.define('search-box');
```

</template>
</CodeComparison>

**What changed:** Event handlers use `@event` syntax instead of `onEvent` props. Input value is read from `e.currentTarget` in the `@input` handler — no DOM reference needed. No synthetic event system - the browser's native events are used directly.

## Parent-Child Communication

<CodeComparison>
<template #left>

```jsx
function ColorPicker({ onColorChange }) {
  return (
    <div className="colors">
      <button onClick={() => onColorChange('red')}>Red</button>
      <button onClick={() => onColorChange('blue')}>Blue</button>
    </div>
  );
}

function App() {
  const [color, setColor] = useState('');

  return (
    <div>
      <ColorPicker onColorChange={setColor} />
      <p>Selected: {color}</p>
    </div>
  );
}
```

</template>
<template #right>

**color-picker.html**

```html
<div class="colors">
  <button @click="{selectColor('red')}">Red</button>
  <button @click="{selectColor('blue')}">Blue</button>
</div>
```

**color-picker.ts**

```typescript
import { WebUIElement } from '@microsoft/webui-framework';

export class ColorPicker extends WebUIElement {
  selectColor(color: string): void {
    this.$emit('color-change', { detail: { color } });
  }
}

ColorPicker.define('color-picker');
```

**theme-app.html**

```html
<template shadowrootmode="open"
  @color-change="{onColorChange(e)}"
>
  <color-picker></color-picker>
  <p>Selected: {{currentColor}}</p>
</template>
```

**theme-app.ts**

```typescript
import { WebUIElement, observable } from '@microsoft/webui-framework';

export class ThemeApp extends WebUIElement {
  @observable currentColor = '';

  onColorChange(e: CustomEvent): void {
    this.currentColor = e.detail.color;
  }
}

ThemeApp.define('theme-app');
```

</template>
</CodeComparison>

**What changed:** React passes callback props down; WebUI uses native Custom Events that bubble up through the DOM. The child emits an event with `this.$emit()`, and the parent catches it with `@event` syntax on the component tag. Components are fully decoupled - the child doesn't reference the parent.

## Styling

<CodeComparison>
<template #left>

```jsx
import styled from 'styled-components';

const Card = styled.div`
  padding: 1rem;
  border: 1px solid #e0e0e0;
  border-radius: 8px;
`;

const Title = styled.h3`
  color: #333;
  margin: 0 0 0.5rem 0;
`;

function ProductCard({ name, price }) {
  return (
    <Card>
      <Title>{name}</Title>
      <p>${price}</p>
    </Card>
  );
}
```

</template>
<template #right>

**product-card.html**

```html
<div class="card">
  <h3>{{name}}</h3>
  <p>${{price}}</p>
</div>
```

**product-card.css**

```css
.card {
  padding: 1rem;
  border: 1px solid #e0e0e0;
  border-radius: 8px;
}

h3 {
  color: #333;
  margin: 0 0 0.5rem 0;
}
```

</template>
</CodeComparison>

**What changed:** CSS-in-JS becomes a plain CSS file. Shadow DOM provides the style encapsulation that CSS-in-JS libraries simulate with generated class names. Styles cannot leak in or out of the component. No JavaScript runtime cost for styling.

## Component Composition

<CodeComparison>
<template #left>

```jsx
function UserProfile({ user }) {
  return (
    <div className="profile">
      <img src={user.avatar} alt={user.name} />
      <h2>{user.name}</h2>
      <p>{user.bio}</p>
      {user.isAdmin && <AdminBadge />}
      <ul>
        {user.skills.map((skill) => (
          <li key={skill}>{skill}</li>
        ))}
      </ul>
    </div>
  );
}
```

</template>
<template #right>

**user-profile.html**

```html
<div class="profile">
  <img src="{{user.avatar}}" alt="{{user.name}}" />
  <h2>{{user.name}}</h2>
  <p>{{user.bio}}</p>
  <if condition="user.isAdmin">
    <admin-badge></admin-badge>
  </if>
  <ul>
    <for each="skill in user.skills">
      <li>{{skill}}</li>
    </for>
  </ul>
</div>
```

</template>
</CodeComparison>

**What changed:** JSX expressions (`&&`, `.map()`, template literals) become HTML directives (`<if>`, `<for>`, <code v-pre>{{}}</code>). The template reads like HTML with declarative annotations, not JavaScript with embedded markup.

## FAST Alternative

WebUI supports two hydration plugins. The examples above use `@microsoft/webui-framework`. If your team uses the [FAST](https://fast.design/) ecosystem, the `--plugin=fast` option provides an alternative:

```bash
webui build ./src --out ./dist --plugin=fast
```

The **template syntax is identical** - `<if>`, `<for>`, <code v-pre>{{}}</code>, and `@click` work the same way in both plugins. The difference is in the TypeScript component class:

<CodeComparison left-label="WebUI Framework" right-label="FAST">
<template #left>

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

</template>
<template #right>

```typescript
import { FASTElement, attr, observable } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

export class MyCounter extends RenderableFASTElement(FASTElement) {
  @attr label = 'Count';
  @observable count = 0;

  increment(): void {
    this.count += 1;
  }

  prepare(): void {
    // Manually read state from pre-rendered DOM
    this.count = Number(this.shadowRoot?.querySelector('span')?.textContent ?? 0);
  }
}

MyCounter.define({ name: 'my-counter', template: /* ... */ });
```

</template>
</CodeComparison>

| | WebUI Framework | FAST |
|---|---|---|
| **State seeding** | Automatic from SSR markers | Manual in `prepare()` |
| **Update model** | Targeted path-indexed | Full observable chain |
| **Package** | `@microsoft/webui-framework` | `@microsoft/fast-html` + `@microsoft/fast-element` |
| **Best for** | SSR-first apps, minimal JS | Complex client interactivity, existing FAST projects |

## Architecture Comparison

### React: client-side rendering pipeline

```
Build → JS Bundle → Browser downloads → Parse JS → Execute → Fetch data → Render
```

Every component ships JavaScript. The page is blank until the bundle loads, parses, and executes.

### WebUI: server-rendered with interactive islands

```
Build → Protocol Binary → Server renders HTML → Browser displays immediately
                                               → JS loads only for interactive islands
```

Static content is visible instantly. Only components that need interactivity ship JavaScript.

| Metric | React SPA | WebUI Islands |
|--------|-----------|---------------|
| First paint | After JS bundle loads | Immediate (server HTML) |
| JavaScript shipped | All components | Only interactive islands |
| Server runtime | Node.js with V8 | Rust binary, no JS runtime |
| Component encapsulation | Convention (CSS Modules, etc.) | Native (Shadow DOM) |

## Summary

Moving from React to WebUI means:

1. **Separate files** instead of JSX - HTML, CSS, and TypeScript each have their own file
2. **Declarative directives** instead of JavaScript expressions - `<if>`, `<for>`, <code v-pre>{{}}</code>
3. **Native Web Components** instead of framework components - Shadow DOM, Custom Elements
4. **Server-first rendering** instead of client-first - content is visible before any JavaScript loads
5. **Targeted updates** instead of virtual DOM diffing - only the specific DOM nodes bound to a changed property update
6. **Custom Events** instead of callback props - components communicate through the standard DOM event system
