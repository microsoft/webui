# Best Practices

This page covers proven patterns and common pitfalls when building WebUI applications. Following these practices will help you write components that render correctly on the server, hydrate efficiently, and stay maintainable as your application grows.

## SSR State Completeness

Every binding in your template should have a corresponding key in the server
state JSON. The handler resolves bindings by looking up keys. A missing text or
attribute binding renders empty; a missing condition identifier is a falsy
operand, so a positive branch is hidden and a negated branch is shown.

**The rule:** check every `{{binding}}`, `<if condition>`, and `<for each>` in your template and ensure the server provides the data.

```html
<!-- Template bindings -->
<h1>{{product.name}}</h1>
<if condition="product.inStock">
  <span>In Stock ({{product.quantity}} available)</span>
</if>
<for each="review in product.reviews">
  <p>{{review.text}} - {{review.author}}</p>
</for>
```

The server state JSON must include all referenced paths:

```json
{
  "product": {
    "name": "Widget Pro",
    "inStock": true,
    "quantity": 42,
    "reviews": [
      { "text": "Great product!", "author": "Alice" },
      { "text": "Works perfectly.", "author": "Bob" }
    ]
  }
}
```

Missing keys produce silent failures - the template renders without errors, but content is absent. If your initial page load is missing data, check the server state first.

## Use Template Expressions, Not Shadow Observables

A common mistake is creating `@observable` properties that simply mirror a condition already expressible in the template. This adds unnecessary state, introduces synchronization bugs, and requires extra server state keys.

❌ **Anti-pattern - shadow observable:**

```typescript
@observable items: Item[] = [];
@observable hasItems = false; // mirrors items.length > 0

onItemsChanged(): void {
  this.hasItems = this.items.length > 0; // manual sync
}
```

```html
<if condition="hasItems">...</if>
```

Now you need `hasItems` in the server state JSON, and you must keep it synchronized with `items` on the client.

✅ **Correct - use a template expression:**

```html
<if condition="items.length">...</if>
```

The condition evaluator handles this directly. No extra property. No synchronization. The server provides `items` and the expression evaluates truthiness from its length.

### Supported condition expressions

The template condition evaluator supports:

- **Dot paths:** `user.profile.name`
- **Truthiness:** `items.length` (zero is falsy)
- **Negation:** `!isLoading`
- **Comparisons:** `count > 0`, `status == 'active'`
- **Compound:** `isLoggedIn && hasPermission`

Use these instead of creating derived observables.

## Boolean Attributes

### Use `@attr({ mode: 'boolean' })` for True/False State

Boolean attributes follow the HTML spec: **present means true, absent means false**. There is no `"false"` value - a string `"false"` is truthy.

```typescript
@attr({ mode: 'boolean' }) disabled = false;
@attr({ mode: 'boolean' }) checked = false;
```

Bind boolean attributes in templates with the `?` prefix:

```html
<button ?disabled="{{isLoading}}">Submit</button>
<input type="checkbox" ?checked="{{isSelected}}" />
```

### The String `"false"` Trap

<webui-blockquote appearance="danger" title="Danger" icon="🚨">

Never use the string `"false"` for boolean attributes. In HTML and JavaScript, a non-empty string is truthy.

</webui-blockquote>

```html

<!-- ❌ WRONG - "false" is a truthy string, button will be disabled -->
<button disabled="false">Submit</button>

<!-- ✅ CORRECT - use ?attr binding with a boolean value -->
<button ?disabled="{{isLoading}}">Submit</button>
```

In your server state JSON, use actual booleans:

```json
{
  "isLoading": false,
  "isSelected": true
}
```

## Observable Truthiness in `<if>` Conditions

The `<if>` directive evaluates conditions with JavaScript-like truthiness, with one important exception: empty collections. The server evaluator treats an empty array or object as falsy, while the compiled client condition is plain `!!value`, where both are truthy. Understanding these rules prevents subtle rendering bugs.

| Value | Truthy? | Notes |
|-------|---------|-------|
| `true` | ✅ Yes | |
| `false` | ❌ No | |
| `1`, `42`, `-1` | ✅ Yes | Any non-zero number |
| `0` | ❌ No | |
| `"hello"` | ✅ Yes | Any non-empty string |
| `""` | ❌ No | Empty string |
| `"false"` | ✅ Yes ⚠️ | Non-empty string - this is truthy! |
| `"0"` | ✅ Yes ⚠️ | Non-empty string - this is truthy! |
| `[]` (empty array) | ⚠️ Differs | Falsy on the server, truthy on the client - never test it directly |
| `{}` (empty object) | ⚠️ Differs | Falsy on the server, truthy on the client - never test it directly |
| `[].length` → `0` | ❌ No | Use `.length` to check for empty arrays |

### Common patterns

```html
<!-- Check if an array has items -->
<if condition="items.length">
  <p>Showing {{items.length}} results</p>
</if>

<!-- Check a boolean flag -->
<if condition="isLoggedIn">
  <user-menu></user-menu>
</if>

<!-- Negate a condition -->
<if condition="!isLoading">
  <div class="content">...</div>
</if>
```

<webui-blockquote appearance="tip" title="Tip" icon="💡">

Always use `.length` to check whether an array is empty. Never write `<if condition="items">`: the server treats `[]` as falsy while the client treats it as truthy, so server-rendered output and hydration can disagree. `items.length` is `0` for an empty array, which is falsy on both sides.

</webui-blockquote>

## React Patterns to Avoid

If you're coming from React, some familiar patterns work against WebUI's declarative template model. Here are the most common ones and their WebUI equivalents.

### 1. Array Rebuild for Single-Property Toggle

❌ **React habit - rebuild the array to toggle a property:**

```typescript
// Rebuilds the entire array to toggle one item
toggleItem(id: string): void {
  this.items = this.items.map(item =>
    item.id === id ? { ...item, selected: !item.selected } : item
  );
}
```

✅ **WebUI approach - use a template condition:**

```typescript
toggleItem(id: string): void {
  const item = this.items.find(i => i.id === id);
  if (item) {
    item.selected = !item.selected;
  }
}
```

```html
<for each="item in items">
  <div ?data-selected="{{item.selected}}">{{item.name}}</div>
</for>
```

### 2. Changed-Callback Chains

❌ **React habit - useEffect chains to sync derived state:**

```typescript
@observable items: Item[] = [];
@observable filteredItems: Item[] = [];
@observable count = 0;

// Cascading updates
onItemsChanged(): void {
  this.filteredItems = this.items.filter(i => i.active);
  this.count = this.filteredItems.length;
}
```

✅ **WebUI approach - let the template handle derived values:**

```html
<if condition="items.length">
  <for each="item in items">
    <if condition="item.active">
      <div>{{item.name}}</div>
    </if>
  </for>
</if>
```

No intermediate state. The template composes conditions directly.

### 3. Shadow Observables

❌ **React habit - derived state stored in separate variables:**

```typescript
@observable firstName = '';
@observable lastName = '';
@observable fullName = '';  // shadow of firstName + lastName

onNameChanged(): void {
  this.fullName = `${this.firstName} ${this.lastName}`;
}
```

✅ **WebUI approach - use expressions or provide from server state:**

```html
<!-- Bind both values directly -->
<span>{{firstName}} {{lastName}}</span>
```

Or, if you need a single computed value on the client, compute it in an event handler and store it as an `@observable`:

```typescript
@observable fullName = '';

private updateFullName(): void {
  this.fullName = `${this.firstName} ${this.lastName}`;
}
```

### 4. Manual DOM Sync via `w-ref`

❌ **Anti-pattern - using refs to manually sync DOM attributes:**

```typescript
@observable isActive = false;

onIsActiveChanged(): void {
  if (this.buttonRef) {
    this.buttonRef.setAttribute('aria-pressed', String(this.isActive));
    this.buttonRef.classList.toggle('active', this.isActive);
  }
}
```

✅ **WebUI approach - declarative attribute binding:**

```html
<button
  ?aria-pressed="{{isActive}}"
  ?data-active="{{isActive}}"
  @click="{toggle()}"
>
  {{label}}
</button>
```

```css
:host button[data-active] {
  background: #0078d4;
  color: white;
}
```

### 5. Manual `classList` Toggle

❌ **Anti-pattern - toggling classes imperatively:**

```typescript
@observable theme = 'light';

onThemeChanged(): void {
  this.containerRef.classList.toggle('dark', this.theme === 'dark');
  this.containerRef.classList.toggle('light', this.theme === 'light');
}
```

✅ **WebUI approach - use `@observable` + data attributes:**

```typescript
@observable theme = 'light';
```

```html
<template shadowrootmode="open">
  <div ?data-dark="{{theme == 'dark'}}">
    <slot></slot>
  </div>
</template>
```

```css
:host div[data-dark] {
  background: #1a1a1a;
  color: #f0f0f0;
}
```

## Route-Scoped State

Each SSR route handler should return only the data that its template actually binds to. Sending the entire application state on every route wastes bandwidth and slows down rendering.

❌ **Anti-pattern - returning full app state:**

```json
{
  "user": { "...full profile..." },
  "products": [ "...all 500 products..." ],
  "cart": { "...full cart..." },
  "recommendations": [ "..." ],
  "recentlyViewed": [ "..." ],
  "notifications": [ "..." ]
}
```

This might be 240 KB for a product detail page that only needs the product and user name.

✅ **Correct - return only what the view binds to:**

```json
{
  "user": { "name": "Alice" },
  "product": {
    "name": "Widget Pro",
    "price": 29.99,
    "description": "A professional widget.",
    "inStock": true,
    "reviews": [
      { "text": "Great!", "author": "Bob", "rating": 5 }
    ]
  }
}
```

This is roughly 15 KB - the handler renders faster, the network transfer is smaller, and the client parses less JSON.

**Rule:** For each route, look at the template bindings and return exactly those keys. Nothing more.

## Light DOM vs Shadow DOM

Shadow is the default fallback for unwrapped component content. Build with
`--dom light` to make unwrapped components Light; the compiler scopes their
ordinary paired CSS and component-local `<style>` blocks, so authors still use
`:host` and do not need global component styles. Selector relationships inside
`:is()`, `:where()`, `:not()`, and `:has()` are scoped recursively, and named
`@layer` values receive a component namespace.

Raw `{{{html}}}` bindings use the general `@scope` enclosure because their
elements are unknown at build time. Avoid selector functions in those
components: browser scope limits do not stop relational selectors from
inspecting external ancestors or nested component internals, so WebUI rejects
that combination. Use simpler selectors or keep that component Shadow.

### Performance Comparison

Light generally reduces ShadowRoot/stylesheet-object count and improves
bundling, SSR throughput, and document completion. Shadow supplies a native CSS
tree boundary and can be faster when large repeated components recalculate
styles frequently. Choose based on workload rather than assuming either mode is
always faster.
| Memory per Component | Baseline | Lower | No shadow root overhead |

### When to Use Each

**Light DOM** (`--dom light`, unwrapped templates):

- Mostly static or moderately styled application composition
- Components that benefit from normal inheritance and a flatter DOM
- Components that benefit from shared bundled CSS

**Shadow DOM**:

- Components that use native `<slot>` composition
- Third-party components embedded in unknown host pages
- Components that specifically require a native Shadow boundary
- CSS-heavy, highly repeated components with frequent class/attribute changes

### Authoring Shadow DOM

Shadow is the default. In a `--dom light` build, wrap a component that should
remain Shadow in the sole top-level element:

```html
<template shadowrootmode="open">
  <slot></slot>
</template>
```

Only `open` is supported. A closed root or invalid wrapper placement/value is a
build error. Native `<slot>` is valid in effective Shadow components and fails
in effective Light components.

## Summary

| Practice | Why |
|----------|-----|
| Provide all bound keys in server state | Missing keys silently render empty |
| Use template expressions over shadow observables | Fewer properties, no sync bugs, no extra server state |
| Use `@attr({ mode: 'boolean' })` for true/false | Follows HTML spec, avoids string `"false"` trap |
| Check `.length` for empty arrays | Server and client disagree on bare `[]`; `.length` of `0` is falsy on both |
| Return route-scoped state | Smaller payloads, faster rendering |
| Prefer declarative bindings over imperative DOM manipulation | Template bindings are reactive and SSR-compatible |
| Use `--dom light` only when its composition/network wins fit the app | Avoids trading native CSS isolation for the wrong workload |
| Author an open wrapper for Shadow islands in a Light build | Keeps slots, encapsulation, and CSS-heavy components tree-local |
| Put native `<slot>` only in an effective Shadow component | Native slots do not work in Light DOM |
