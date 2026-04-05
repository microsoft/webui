# `<if>` Conditional Directive

The `<if>` directive allows you to conditionally render content based on a boolean expression.

## Basic Usage

```html
<if condition="isLoggedIn">
  <div class="welcome">Welcome back, {{username}}!</div>
</if>
```

In this example, the welcome message will only be rendered if `isLoggedIn` evaluates to `true` in the current state.

## Condition Expressions

The `condition` attribute supports various types of expressions:

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

- Nested conditions are supported but limited to a maximum of 5 logical operators
- You cannot mix different types of logical operators (`&&` and `||`) in the same condition
- Parentheses for grouping are not supported
- The condition is evaluated against the current state object

## Truthiness Rules

The `<if>` condition evaluator follows JavaScript truthiness semantics on both
server (Rust `serde_json::Value`) and client (`!!value`):

| Value | Truthy? | Example |
|-------|---------|---------|
| `true` (boolean) | ✅ Yes | `@observable show = true` |
| `false` (boolean) | ❌ No | `@observable show = false` |
| `0` (number) | ❌ No | `@observable count = 0` |
| `1` or any non-zero number | ✅ Yes | `@observable count = 5` |
| `""` (empty string) | ❌ No | `@observable name = ''` |
| `"hello"` (non-empty string) | ✅ Yes | `@observable name = 'Alice'` |
| `"false"` (string) | ⚠️ **Yes** | Non-empty string is truthy! |
| `[]` (empty array) | - | Use `items.length` instead |
| `null` / missing | ❌ No | Missing state key |

::: warning
The string `"false"` is truthy because it is a non-empty string. Never use
`@observable show = 'false'` with `<if condition="show">`. Use a real boolean:
`@observable show = false`.
:::

### Supported Expression Patterns

| Pattern | Example |
|---------|---------|
| Simple truthiness | `<if condition="isActive">` |
| Negation | `<if condition="!isBlocked">` |
| Dot paths | `<if condition="items.length">` |
| Comparisons | `<if condition="status == 'active'">` |
| Compound AND | `<if condition="isAdmin && isActive">` |
| Compound OR | `<if condition="hasEmail \|\| hasPhone">` |
