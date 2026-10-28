---
title: "Conditional rendering"
description: "Render a region with if conditions, comparisons, and logical operators, using the same truthiness rules on server and client."
---

# `<if>` Conditional Directive

Use `<if condition="...">` to include a region when its condition is truthy.
The directive does not create an extra DOM element.

Its attributes are `condition` and optional `lazy`, whose value is an optional
prefetch trigger. Unsupported attributes, including `id`, are build errors.

## Basic Usage

```html
<if condition="isLoggedIn">
  <div class="welcome">Welcome back, {{username}}!</div>
</if>
```

The welcome message renders when `isLoggedIn` is truthy in the current state.

## Lazy bodies

Add `lazy` to retain an ordinary template body in the protocol without
delivering it with the page while its condition is false. The build emits the
body as a static template pack that the framework loads when the condition
becomes true:

```html
<if condition="settingsReady" lazy>
  <h2>Settings</h2>
  <p>{{displayName}}</p>
</if>
```

`lazy` prefetches the pack when the user interacts with the owning component.
Use `lazy="idle"`, `lazy="load"`, or `lazy="demand"` to prefetch when the
browser is idle, right after startup, or not at all. An initially true condition
still renders on the server. The body keeps its owning template's state, events,
refs, and loop scope; it does not become a component, and DOM-mode rules are
unchanged. Await `this.$rendered()` after turning the condition on before you
use refs inside the body. See [lazy template bodies](../lazy) for setup,
triggers, state loading, and hosting packs.

## Condition Expressions

Write the expression directly in `condition`, without binding braces:

### Simple Identifier

```html
<if condition="isAdmin">
  <!-- Admin-only content -->
</if>
```

### Comparison Operations

```html
<if condition="user.age > 18">
  <!-- Adult content -->
</if>
```

Supported comparison operators:
- `>` (greater than)
- `<` (less than)
- `==` (equal to)
- `!=` (not equal to)
- `>=` (greater than or equal to)
- `<=` (less than or equal to)

### Logical Operations

```html
<if condition="isAdmin && isActive">
  <!-- Active admin content -->
</if>
```

Supported logical operators:
- `&&` (logical AND)
- `||` (logical OR)

### Negation

```html
<if condition="!isBlocked">
  <!-- Content for non-blocked users -->
</if>
```

## Notes and Limitations

- Conditions may nest; each expression supports at most 5 logical operators
- You cannot mix different types of logical operators (`&&` and `||`) in the same condition
- Parentheses for grouping are not supported
- The condition is evaluated against the current state object

## Truthiness Rules

The `<if>` condition evaluator uses the same rules on the server and client.
Scalars follow JavaScript truthiness. Unlike JavaScript, empty arrays and
objects are falsy; nonempty collections are truthy.

| Value | Truthy? | Example |
|-------|---------|---------|
| `true` (boolean) | ✅ Yes | `@observable show = true` |
| `false` (boolean) | ❌ No | `@observable show = false` |
| `0` (number) | ❌ No | `@observable count = 0` |
| `1` or any non-zero number | ✅ Yes | `@observable count = 5` |
| `""` (empty string) | ❌ No | `@observable name = ''` |
| `"hello"` (non-empty string) | ✅ Yes | `@observable name = 'Alice'` |
| `"false"` (string) | ⚠️ **Yes** | Non-empty string is truthy! |
| `[]` (empty array) | ❌ No | `@observable items = []` |
| `{}` (empty object) | ❌ No | `@observable options = {}` |
| Nonempty array or object | ✅ Yes | `[0]` or `{ enabled: false }` |
| `null` / missing identifier | ❌ No | Missing state key |

A missing identifier is a falsy operand, not a failure of the complete
expression. Therefore `<if condition="missingPath">` does not render, while
`<if condition="!missingPath">` does render. This applies to dotted paths in
loop scopes, such as `!item.optionalFlag`.

<webui-blockquote appearance="warning" title="Warning" icon="⚠️">

The string `"false"` is truthy because it is a non-empty string. Never use
`@observable show = 'false'` with `<if condition="show">`. Use a real boolean:
`@observable show = false`.

</webui-blockquote>

Both `<if condition="items">` and `<if condition="items.length">` test whether
an array is nonempty. Use `.length` in comparisons such as
`<if condition="items.length > 1">` when the count matters. Nested collection
paths such as `<if condition="actions.trailing">` use the same rules.

### Supported Expression Patterns

| Pattern | Example |
|---------|---------|
| Simple truthiness | `<if condition="isActive">` |
| Negation | `<if condition="!isBlocked">` |
| Dot paths | `<if condition="items.length">` |
| Comparisons | `<if condition="status == 'active'">` |
| Compound AND | `<if condition="isAdmin && isActive">` |
| Compound OR | `<if condition="hasEmail \|\| hasPhone">` |

Each side of a comparison is a literal (number, quoted string, `true`, `false`)
or a dotted state path. There is no arithmetic and there are no function calls -
`<if condition="currentIndex == items.length - 1">` compares against a state key
literally named `items.length - 1`, never resolves, and is silently false. Send a
precomputed `lastIndex` from the server instead.
