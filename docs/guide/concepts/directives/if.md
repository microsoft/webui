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

## Protocol Output

When the WebUI parser processes an `<if>` directive, it generates a protocol entry like this:

```json
{
  "type": "if",
  "condition": {
    "kind": "identifier",
    "value": "isLoggedIn"
  },
  "streamId": "if-1"
}
```

The content inside the `<if>` directive is stored in a separate stream with the ID specified in `streamId`.
