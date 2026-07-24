# React vs Web Components

This guide compares common UI patterns written in React (imperative, JavaScript-centric) with their WebUI equivalents (declarative, HTML-centric). Each section shows React and WebUI side by side so you can see how familiar patterns translate to a Web Components model.

## Key Differences at a Glance

| | React | WebUI |
|---|---|---|
| **Component model** | JSX functions / classes with virtual DOM | Web Components with Shadow DOM or Light DOM |
| **Rendering** | Client-side or Node.js SSR | Build-time compiled protocol, server-rendered HTML |
| **Template language** | JSX (JavaScript + HTML mixed) | Separate HTML, CSS, and TypeScript files |
| **State management** | `useState`, `useReducer`, context | `@observable` properties with targeted DOM updates |
| **Styling** | CSS-in-JS, CSS Modules, or external | Scoped CSS via Shadow DOM or Global Light DOM via `--dom` and `--css` args |
| **Runtime** | React runtime + ReactDOM in browser | No framework runtime for static content; thin hydration for interactive islands |
| **Interactivity** | Every component ships JavaScript | Only interactive islands ship JavaScript |

## Simple Counter

<code-comparison left-label="React" right-label="WebUI Framework">
<div slot="left">

**my-counter.jsx**

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

</div>
<div slot="right">

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

</div>
</code-comparison>

**What changed:** Template and logic are separated into HTML and TypeScript files. No JSX, no `useState` hook, no `setState` call. The `@observable` decorator makes `count` reactive - when it changes, only the bound DOM nodes update.

## Conditional Rendering

<code-comparison left-label="React" right-label="WebUI Framework">
<div slot="left">

**user-greeting.jsx**

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

</div>
<div slot="right">

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

</div>
</code-comparison>

**What changed:** Conditional logic moves from JavaScript ternary expressions into declarative `<if>` directives. These are evaluated on the server during rendering - no JavaScript is shipped to the browser for static conditionals.

## List Rendering

<code-comparison left-label="React" right-label="WebUI Framework">
<div slot="left">

**todo-list.jsx**

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

</div>
<div slot="right">

**todo-list.html**

```html
<ul>
  <for each="item in items">
    <li key="{{item.id}}">
      <span>{{item.title}}</span>
      <span class="status {{item.state}}">{{item.state}}</span>
    </li>
  </for>
</ul>
```

</div>
</code-comparison>

**What changed:** `Array.map()` with JSX becomes a declarative `<for>`
directive. Like React, reorderable lists can declare stable identity by placing
the compiler-only `key` attribute on the first repeated child. Simple loops may
omit it and reconcile by position. `data-key` and other attributes never act as
implicit keys. This runs on the server and produces static HTML - no JavaScript
array iteration in the browser.

## Event Handling

<code-comparison left-label="React" right-label="WebUI Framework">
<div slot="left">

**search-box.jsx**

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

</div>
<div slot="right">

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

</div>
</code-comparison>

**What changed:** Event handlers use `@event` syntax instead of `onEvent` props. Input value is read from `e.currentTarget` in the `@input` handler — no DOM reference needed. No synthetic event system - the browser's native events are used directly.

## Parent-Child Communication

<code-comparison left-label="React" right-label="WebUI Framework">
<div slot="left">

**color-picker.jsx**

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

</div>
<div slot="right">

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
</div>
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

</div>
</code-comparison>

**What changed:** React passes callback props down; WebUI uses native Custom Events that bubble up through the DOM. The child emits an event with `this.$emit()`, and the parent catches it with `@event` syntax on the component tag. Components are fully decoupled - the child doesn't reference the parent.

## Styling

<code-comparison left-label="React" right-label="WebUI Framework">
<div slot="left">

**product-card.jsx**

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

</div>
<div slot="right">

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

</div>
</code-comparison>

**What changed:** CSS-in-JS becomes a plain CSS file. Shadow DOM provides the style encapsulation that CSS-in-JS libraries simulate with generated class names. Styles cannot leak in or out of the component. No JavaScript runtime cost for styling.

## Component Composition

<code-comparison left-label="React" right-label="WebUI Framework">
<div slot="left">

**user-profile.jsx**

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

</div>
<div slot="right">

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

</div>
</code-comparison>

**What changed:** JSX expressions (`&&`, `.map()`, template literals) become HTML directives (`<if>`, `<for>`, `{{}}`). The template reads like HTML with declarative annotations, not JavaScript with embedded markup.
