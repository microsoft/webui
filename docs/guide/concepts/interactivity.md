# Interactivity

WebUI uses **Islands Architecture** for client-side interactivity. Each Web Component is a self-contained island with its own HTML template, scoped CSS, and TypeScript behavior. Only components that need interactivity ship JavaScript - everything else stays as static server-rendered HTML.

## Component Files

Every interactive component consists of three separate files. Templates are declarative - no JavaScript mixing.

```
my-counter/
├── my-counter.html   ← Template (structure and bindings)
├── my-counter.css    ← Styles (scoped via Shadow DOM)
└── my-counter.ts     ← Behavior (TypeScript class)
```

- **HTML** defines what the component renders and where dynamic values appear
- **CSS** styles the component in isolation - Shadow DOM prevents leaking
- **TypeScript** defines reactive properties, event handlers, and component logic

## The Component Class

Every interactive component extends `WebUIElement` and registers itself as a custom element:

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

The matching template (`my-counter.html`):

```html
<button @click="{increment()}">
  {{label}}: {{count}}
</button>
```

And scoped styles (`my-counter.css`):

```css
:host {
  display: inline-block;
}

button {
  padding: 0.5rem 1rem;
  font-size: 1rem;
  cursor: pointer;
}
```

## The `<template>` Tag

The `<template shadowrootmode="open">` wrapper is **optional** in component HTML files. The build tool auto-injects it when it is not present.

**Without `<template>` (most components):**
```html
<!-- my-counter.html -->
<button @click="{increment()}">{{label}}: {{count}}</button>
```

The framework wraps this in a `<template shadowrootmode="open">` during build.

**With `<template>` (root host events):**
```html
<!-- todo-app.html -->
<template shadowrootmode="open"
  @toggle-item="{onToggleItem(e)}"
  @delete-item="{onDeleteItem(e)}"
>
  <h1>{{title}}</h1>
  <div class="todo-list">
    <for each="item in items">
      <todo-item id="{{item.id}}" title="{{item.title}}"></todo-item>
    </for>
  </div>
</template>
```

When you include the `<template>` tag explicitly, the framework uses yours instead of auto-injecting one. The main reason to include it is to attach **root host events** - event listeners on the shadow root itself that catch events bubbling up from child components (`@toggle-item`, `@delete-item` above). This is the delegated event pattern for parent-child communication.

Decorators define how properties behave and how they connect to the template.

### `@attr` - HTML Attributes

Use `@attr` for values passed from a parent element via HTML attributes. These are part of the component's public API.

**String mode** (default):

```typescript
@attr label = 'Default Label';
```

```html
<my-counter label="Items"></my-counter>
```

**Boolean mode** - attribute presence means `true`, absence means `false`:

```typescript
@attr({ mode: 'boolean' }) disabled = false;
```

```html
<!-- disabled = true -->
<my-button disabled></my-button>

<!-- disabled = false -->
<my-button></my-button>
```

### `@observable` - Reactive State

Use `@observable` for internal state that changes over time. When an observable value changes, the framework automatically updates any template bindings that reference it.

```typescript
@observable count = 0;
@observable items: Item[] = [];
@observable isOpen = false;
```

Observable changes are **synchronous and targeted** - only the specific DOM nodes bound to the changed property are updated.

### Derived State

For derived values like "has items?" or "total count", use template expressions directly instead of computed properties:

```html
<!-- Use dot-path expressions in the template -->
<if condition="items.length">
  <span>{{items.length}} items</span>
</if>
```

The condition evaluator supports dot paths (`items.length`), comparisons (`count > 0`), truthiness, and negation (`!isEmpty`). This keeps derived state declarative and works on both server and client.

For complex derived state that can't be expressed in template syntax, compute it on the server and provide it in the JSON state, or compute it in an event handler and store it in an `@observable`.

## Template Syntax for Interactivity

### Reactive Text

Use double curly braces to bind property values into the template:

```html
<span>{{label}}: {{count}}</span>
<p>Hello, {{user.name}}!</p>
```

### Event Binding

Attach event handlers with `@event` syntax:

```html
<!-- Call a method -->
<button @click="{increment()}">Add</button>

<!-- Access the event object -->
<input @keydown="{onKeydown(e)}" />

<!-- Pass repeat-scope values and literals -->
<for each="item in items">
  <button @click="{selectItem(item.id, 'details', e)}">
    {{item.name}}
  </button>
</for>

<!-- Multiple events on one element -->
<div @mouseenter="{onHover()}" @mouseleave="{onLeave()}">
  Hover me
</div>
```

Event handlers use method-call syntax only. Arguments can be:

- `e` for the native DOM event
- Dotted component or repeat-scope paths such as `item.id`
- String, number, boolean, and `null` literals

General JavaScript expressions and nested function calls are not parsed in
templates. Compute those values in the component class or pass a supported path.

### DOM References

Use `w-ref` to get a direct reference to a DOM element:

```html
<input w-ref="inputEl" type="text" />
<button @click="{focusInput()}">Focus</button>
```

```typescript
inputEl!: HTMLInputElement;

focusInput(): void {
  this.inputEl.focus();
}
```

### Conditional Rendering

Render content based on expressions:

```html
<if condition="count > 0">
  <p>You have {{count}} items.</p>
</if>

<if condition="!isLoggedIn">
  <a href="/login">Sign in</a>
</if>
```

### Boolean Attributes

Toggle HTML attributes with the `?` prefix:

```html
<button ?disabled="{{isLoading}}">Submit</button>
<input ?checked="{{isSelected}}" type="checkbox" />
<details ?open="{{isExpanded}}">...</details>
```

### List Rendering

Iterate over arrays with `<for>`:

```html
<ul>
  <for each="item in items">
    <li>{{item.name}} - {{item.price}}</li>
  </for>
</ul>
```

## Event Handling Patterns

### Direct Method Calls

The simplest pattern - call a method when an event fires:

```typescript
@observable count = 0;

increment(): void {
  this.count += 1;
}
```

```html
<button @click="{increment()}">+1</button>
```

### Using the Event Object

Access the native DOM event by passing `e`:

```typescript
onKeydown(e: KeyboardEvent): void {
  if (e.key === 'Enter') {
    this.submit();
  }
}
```

```html
<input @keydown="{onKeydown(e)}" />
```

### Passing Values from Repeats

Handlers inside a `<for>` block can receive the current item through a dotted
path. The framework captures the active repeat scope during hydration and
resolves the argument when the event fires:

```typescript
selectItem(id: string, e: MouseEvent): void {
  e.preventDefault();
  this.selectedId = id;
}
```

```html
<for each="item in items">
  <button @click="{selectItem(item.id, e)}">
    {{item.title}}
  </button>
</for>
```

### Custom Events and Parent-Child Communication

Components communicate upward by emitting custom events with `this.$emit()`:

**Child component** (`color-picker.ts`):

```typescript
export class ColorPicker extends WebUIElement {
  @observable selectedColor = '';

  selectColor(color: string): void {
    this.selectedColor = color;
    this.$emit('color-change', { detail: { color } });
  }
}
```

**Parent template** catches the event:

```html
<color-picker @color-change="{onColorChange(e)}"></color-picker>
<p>Selected: {{currentColor}}</p>
```

**Parent class** handles the event:

```typescript
onColorChange(e: CustomEvent): void {
  this.currentColor = e.detail.color;
}
```

This pattern keeps components decoupled - the child doesn't know who is listening, and the parent reacts declaratively.

## Styling

CSS is scoped to each component via Shadow DOM. Styles in one component cannot leak into or be affected by another.

### The `:host` Selector

Style the component's root element with `:host`:

```css
:host {
  display: block;
  padding: 1rem;
  border: 1px solid #e0e0e0;
}
```

### Attribute-Based Styling

Style the component differently based on its attributes with `:host([attr])`:

```css
:host([variant="primary"]) {
  background: #0078d4;
  color: white;
}

:host([disabled]) {
  opacity: 0.5;
  pointer-events: none;
}
```

### Scoping Rules

- Styles defined in a component's `.css` file only apply inside that component's shadow root
- External page styles do not penetrate into the component
- No CSS-in-JS - styles stay in `.css` files, separate from behavior
- Use CSS custom properties (`--my-color`) to allow external theming

## SSR + Interactivity Lifecycle

Understanding the lifecycle helps you write components that work correctly from the first paint through interactive use.

### 1. Server renders HTML

The handler renders the component's template using JSON state data. No JavaScript runs. The output includes Declarative Shadow DOM:

```html
<my-counter>
  <template shadowrootmode="open">
    <style>/* scoped styles */</style>
    <button>Count: 0</button>
  </template>
</my-counter>
```

### 2. Browser displays content

The browser parses the HTML and renders it immediately. The user sees fully styled content - no loading spinner, no blank page, no flash of unstyled content.

### 3. JavaScript loads and components hydrate

The framework detects the existing Declarative Shadow DOM roots and upgrades elements in place:

- Bindings are wired to class properties
- Event handlers are attached
- `@observable` properties become reactive
- The component is now interactive

### 4. User interacts

From this point on, interactions are handled entirely on the client. Changes to `@observable` properties trigger targeted DOM updates without a server round-trip.

## When NOT to Hydrate

Not every component needs JavaScript. Hydrating a component that has no interactivity adds unnecessary bytes and processing time.

**Skip hydration for:**

- **Static content pages** - about, docs, marketing, legal. The server renders them perfectly.
- **Read-only data displays** - lists, tables, cards with no user interaction. Server-rendered HTML is sufficient.
- **Layout components** - headers, footers, sidebars with only links. Standard `<a>` tags work without JS.

**Hydrate when a component needs:**

- Event handlers (`@click`, `@keydown`, `@input`)
- Reactive state updates (`@observable` properties that change)
- User input handling (forms, search, filters)
- Client-side data manipulation (sorting, filtering, pagination)

The goal is minimal JavaScript: hydrate only what the user will interact with, and let the server handle everything else.
