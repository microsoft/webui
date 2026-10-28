---
description: Avoid common WebUI authoring mistakes in initial state, observable updates, attributes, refs, and DOM mode.
---

# Best Practices

Use this as an authoring checklist. For a complete setup, follow
[getting-started lessons](/start/#start-here); for optimization choices, use the
[performance guide](/build/performance).

## SSR State Completeness

Trace each initial binding back to server state or a component attribute.
The server does not execute TypeScript field initializers or computed getters.

```html
<h1>{{product.name}}</h1>
<if condition="product.inStock">
  <p>{{product.quantity}} available</p>
</if>
```

```json
{
  "product": {
    "name": "Desk lamp",
    "inStock": true,
    "quantity": 12
  }
}
```

Missing text paths render empty; a missing condition identifier is falsy.
A whole-binding attribute is omitted for missing or `null` values, while
`""`, `0`, and `false` remain values. Do not rely on these fallbacks to catch
spelling mistakes.

## Use Template Expressions, Not Shadow Observables

Do not maintain both `items` and `hasItems` when the latter only mirrors
`items.length`. Bind the condition directly:

```html
<if condition="items.length">
  <p>{{items.length}} results</p>
</if>
```

### Supported condition expressions

Use paths, negation, comparisons, and limited logical operators:

```html
<if condition="isLoggedIn && hasPermission">...</if>
<button ?disabled="{{currentIndex == 0}}">Previous</button>
```

Text bindings only look up paths. Arithmetic, function calls, ternaries,
grouping parentheses, mixed `&&`/`||`, and more than five logical operators
are not supported. Compute more complex initial display values on the server,
then update them explicitly if browser interactions change their inputs.

## Boolean Attributes

### Use `@attr({ mode: 'boolean' })` for True/False State

For a custom element's boolean API:

```typescript
@attr({ mode: 'boolean' }) disabled = false;
```

For native or custom HTML boolean attributes, bind presence with `?`:

```html
<button ?disabled="{{isSaving}}">Save</button>
<input type="checkbox" ?checked="{{isSelected}}">
```

### The String `"false"` Trap

`disabled="false"` still disables a button because the attribute is present.
Use actual JSON booleans, not strings.

ARIA attributes are different: they often need a literal `"true"` or `"false"`
value. For example, use <code v-pre>aria-pressed="{{isActive}}"</code>, not
`?aria-pressed`.

## Observable Truthiness in `<if>` Conditions

WebUI uses the same rules on the server and browser. Unlike JavaScript, empty
arrays and objects are falsy.

| Value | Truthy? |
| --- | --- |
| `true`, a nonzero number, a nonempty string | Yes |
| `"false"` or `"0"` | Yes; both are nonempty strings |
| `false`, `0`, `""`, `null`, a missing path | No |
| `[]` or `{}` | No |
| A nonempty array or object | Yes, even if its contents are falsy |

### Common patterns

```html
<if condition="items">...</if>
<if condition="items.length > 1">...</if>
<if condition="!isLoading">...</if>
```

Use `.length` when comparing counts, and explicit booleans for independent
conditions.

<span id="react-patterns-to-avoid"></span>

## State and DOM Pitfalls

These are WebUI-specific rules, not rules against React. Immutable state
updates are valid in WebUI too. Choose the update that matches the observable
contract and the operation.

### 1. Array Rebuild for Single-Property Toggle

`@observable` observes assignments to the property; it does not automatically
observe arbitrary nested mutations. This update changes one item and then
notifies the setter:

```typescript
toggleItem(id: string): void {
  const item = this.items.find(item => item.id === id);
  if (!item) return;
  item.selected = !item.selected;
  this.items = [...this.items];
}
```

An immutable `.map()` replacement also works. Do not remove the reassignment
and assume the nested write is reactive. Use stable repeat keys to preserve
row identity when membership or order changes.

### 2. Changed-Callback Chains

A property callback is named `<property>Changed(oldValue, newValue)`, such as
`itemsChanged`. It runs after mounting for initial state, then synchronously on
live assignments; template updates are batched separately.

Use these callbacks for effects that genuinely follow an assignment. Avoid
chains of callbacks just to keep multiple copies of derived state in sync.
Compute the initial value on the server and update it with the operation that
changes its inputs, as the [Todo count](/start/todo-app#6-client-side-component-classes)
does.

### 3. Shadow Observables

If a value is display-only, keep it in server state. If two paths can render
directly, do not add another observable just to join them:

```html
<span>{{firstName}} {{lastName}}</span>
```

Not every derived value can be expressed in a template. A formatted total or
filtered count can be legitimate state; supply its initial value and keep its
later updates together.

### 4. Manual DOM Sync via `w-ref`

Keep state in properties, not in DOM text or attributes. Bind what the UI should
display:

```html
<button
  aria-pressed="{{isActive}}"
  ?data-active="{{isActive}}"
  @click="{toggle()}"
>
  Favorite
</button>
```

Use a braced ref such as `w-ref="{inputEl}"` for `focus()`, `showModal()`, or
another imperative browser API. Use `@input` to read user input from the event,
not a ref to synchronize application state manually.

### 5. Manual `classList` Toggle

Bind a styling attribute and let CSS respond:

```css
button[data-active] {
  font-weight: 700;
}
```

Keep layout, transitions, and animations in CSS. Honor
`prefers-reduced-motion` when adding motion.

## Route-Scoped State

Prepare the data the current page needs, not every application collection.
Projection manifests can reduce browser hydration state, but do not replace
server data selection or access control.

Never put secrets in render state. See
[Build-Time State Projection](/build/hydration#build-time-state-projection)
for the optional payload optimization.

## Light DOM vs Shadow DOM

Use Light DOM when shared styles and document composition are intentional.
Its CSS is global in its containing tree. Use Shadow DOM for native slots,
host selectors, or a style boundary.

Unwrapped components default to Shadow. A sole bare `<template>` selects
Light; a sole open wrapper keeps a component Shadow even in a `--dom light`
build:

```html
<template shadowrootmode="open">
  <slot></slot>
</template>
```

`<slot>`, `:host`, `:host-context`, and `::slotted` are rejected in effective
Light components. A policy wrapper with `w-render` or `w-hydrate` does not
itself select a DOM mode.

## Summary

Before shipping a component:

- Verify its initial display with JavaScript disabled.
- Provide initial data on the server; do not derive it in a constructor.
- Add browser behavior and observables only where used.
- Notify observable setters when changing collections.
- Use native controls, labeled inputs, template bindings, and stable keys.
- Choose Light or Shadow DOM deliberately, and keep browser state public.
