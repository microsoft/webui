# State Management

WebUI uses JSON as its state format. At render time, you pass a JSON object to the handler, and template bindings resolve values from that object using dotted paths.

## State Structure

State is a flat or nested JSON object. Template bindings reference values using dot notation:

```json
{
  "title": "My App",
  "user": {
    "name": "Alice",
    "role": "admin",
    "profile": {
      "avatar": "/img/alice.png"
    }
  },
  "items": [
    { "id": "1", "label": "First", "done": false },
    { "id": "2", "label": "Second", "done": true }
  ]
}
```

```html
<h1>{{title}}</h1>
<img src="{{user.profile.avatar}}" alt="{{user.name}}" />
```

## Path Resolution

The handler resolves paths using `find_value_by_dotted_path`. Supported patterns:

| Pattern | Example | Resolves to |
|---------|---------|-------------|
| Simple property | `title` | `"My App"` |
| Nested property | `user.profile.avatar` | `"/img/alice.png"` |
| Array index | `items.0.label` | `"First"` |
| Array length | `items.length` | `2` |

Paths are resolved at render time. If a path doesn't exist in the state, the handler reports an error with the missing path name.

## State in Loops

Inside a `<for>` directive, each iteration creates a scoped state context. The loop item's properties are accessible directly — no prefix needed:

```html
<for each="item in items">
  <!-- "item.label" resolves to the current item's label -->
  <p>{{item.label}}</p>
  
  <!-- Global state is still accessible -->
  <span>by {{user.name}}</span>
</for>
```

### Scoping Rules

- **Local state** (the current loop item) takes precedence over global state
- **Global state** is always accessible as a fallback
- **Nested loops**: only the innermost loop's item is accessible by its moniker. Global state is available as a fallback.
- **Components inside loops**: access the current item's fields directly without the item moniker. For example, if iterating with `each="contact in contacts"`, a `<contact-card>` component can use `{{name}}` instead of `{{contact.name}}`.

```html
<for each="category in categories">
  <h2>{{category.name}}</h2>
  <for each="product in category.products">
    <!-- "product.name" is the inner loop item -->
    <!-- "category.name" is NOT accessible here — only the innermost item -->
    <!-- "title" still resolves from global state -->
    <p>{{product.name}} — {{product.price}}</p>
  </for>
</for>
```

## State in Conditions

`<if>` directives can reference both local (loop) and global state in the same condition:

```html
<for each="item in items">
  <if condition="item.done && showCompleted">
    <span class="done">{{item.label}}</span>
  </if>
</for>
```

Here, `item.done` comes from the loop item and `showCompleted` comes from global state.

## Designing State for WebUI

### Keep it flat where possible

Deeply nested state works, but adds path traversal cost. Prefer flat structures for frequently accessed values:

```json
// ✅ Preferred — flat access
{
  "userName": "Alice",
  "userRole": "admin"
}

// ⚠️ Works but deeper path resolution
{
  "user": { "profile": { "name": "Alice" } }
}
```

### Structure collections as arrays of objects

The `<for>` directive iterates over arrays. Each item should be a self-contained object with all the data the template needs:

```json
{
  "contacts": [
    { "id": "1", "name": "Alice", "email": "alice@example.com", "avatar": "/img/alice.png" },
    { "id": "2", "name": "Bob", "email": "bob@example.com", "avatar": "/img/bob.png" }
  ]
}
```

### Provide all state upfront

Unlike client-side frameworks that fetch data on mount, WebUI renders in a single pass. The state object must contain everything the template needs for first render. Missing values produce errors, not empty strings.

```json
// ✅ Complete — every binding has data
{
  "title": "Contacts",
  "contacts": [...],
  "showSearch": true,
  "emptyMessage": "No contacts found"
}

// ❌ Missing — "emptyMessage" binding will error
{
  "title": "Contacts",
  "contacts": [...]
}
```

### Use boolean flags for conditionals

`<if>` conditions evaluate against state values. Use explicit boolean flags rather than relying on complex expressions:

```json
{
  "isAdmin": true,
  "hasItems": true,
  "showBanner": false
}
```

```html
<if condition="isAdmin">
  <div class="admin-panel"></div>
</if>
<if condition="!hasItems">
  <p>{{emptyMessage}}</p>
</if>
```

## HTML Escaping

By default, signal values are HTML-escaped to prevent XSS:

| Syntax | Escaping | Use case |
|--------|----------|----------|
| `{{value}}` | Escaped | User-provided text, names, labels |
| `{{{value}}}` | Raw (unescaped) | Pre-sanitized HTML content |

> ⚠️ Never use triple braces for user input or URL parameters. An attacker could inject `<script>` tags.

## Learn More

- [Signals](/guide/concepts/directives/signals) — Template binding syntax
- [For loops](/guide/concepts/directives/for) — Iterating over collections
- [If conditions](/guide/concepts/directives/if) — Conditional rendering
- [Handlers](/guide/concepts/handlers/) — Passing state to the renderer