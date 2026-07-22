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

Paths are resolved at render time. If a path doesn't exist in the state, the Rust handler treats it as a missing value: text and attribute bindings render as empty, and `<if>` conditions using that path evaluate to `false` (the block is not rendered). No error is reported for missing paths by default.

## State in Loops

Inside a `<for>` directive, each iteration creates a scoped state context. Loop items are accessed via their moniker (e.g. `item.label`, `item.done`):

```html
<for each="item in items">
  <!-- Use the moniker to access loop item fields -->
  <p>{{item.label}}</p>
  
  <!-- Global state is still accessible -->
  <span>by {{user.name}}</span>
</for>
```

### Scoping Rules

- **Loop items** are accessed via their moniker (e.g. `item.label`, `item.id`); global state remains accessible alongside them
- **Global state** is always accessible throughout a template
- **Nested loops**: all active loop items remain accessible by their monikers (e.g. `{{category.name}}` inside a `product` loop). Inner loop monikers can shadow global state keys but do not hide outer loop monikers.
- **Components inside loops**: do **not** automatically inherit loop-item fields. Pass the data you need via component attributes (e.g. `&lt;contact-card name="{{contact.name}}"&gt;`), and inside the component template use the attribute names (e.g. `{{name}}`).

```html
<for each="category in categories">
  <h2>{{category.name}}</h2>
  <for each="product in category.products">
    <!-- "product.*" is the inner loop item -->
    <!-- "category.*" is still accessible - outer loop monikers stay in scope -->
    <!-- "title" resolves from global state -->
    <p>{{category.name}}: {{product.name}} - {{product.price}}</p>
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
// ✅ Preferred - flat access
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

Unlike client-side frameworks that fetch data on mount, WebUI renders in a single pass. The state object should contain everything the template needs for first render. Missing values render as empty output (for text and attribute bindings) or evaluate to `false` (for `<if>` conditions) - no error is reported.

```json
// ✅ Complete - every binding has data
{
  "title": "Contacts",
  "contacts": [...],
  "showSearch": true,
  "emptyMessage": "No contacts found"
}

// ⚠️ Partial - "emptyMessage" renders empty, "showSearch" condition evaluates to false
{
  "title": "Contacts",
  "contacts": [...]
}
```

### Keep render state complete; project only browser transport

State projection does not change what the server renderer may read. Keep the
request state complete for every SSR binding, condition, and loop. A validated
bundler manifest only controls which top-level values are copied into the
browser bootstrap block and later route partials.

Without a projection manifest, WebUI sends full state. With one or more
manifests, every scripted component compiled into the protocol must have exact
coverage, and only its proven `@observable` plus `@attr` keys are eligible for
initial bootstrap. See [Hydration](/guide/concepts/hydration#build-time-state-projection).

### SSR State Completeness for Route Pages

When using routing, each route page template has its own bindings. Every
`<for>`, `<if>`, and `{{binding}}` in the page template must have its key
populated in the server state JSON.

```html
<!-- email-detail.html -->
<h2>{{subject}}</h2>
<for each="msg in messages">
  <email-message body="{{msg.body}}"></email-message>
</for>
```

The server must provide both `subject` and `messages`:

```json
{
  "subject": "Q4 Budget Review",
  "messages": [
    { "body": "Please review the attached spreadsheet..." }
  ]
}
```

If `messages` is an empty array `[]`, the `<for>` loop correctly renders
zero items - even if the client would populate it later. The server is
the source of truth for the initial render.

<webui-blockquote appearance="tip" title="Rule of thumb" icon="💡">

Check every `<for>`, `<if>`, and `{{binding}}` in your route page template.
Every key must be present in the server state JSON.

</webui-blockquote>

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

## Generating a State Schema

After building an application, generate a JSON Schema for its render state:

```bash
webui schema ./dist/protocol.bin \
  --title MyAppState > ./dist/state.schema.json
```

When building both artifacts together:

```bash
webui build ./src --out ./dist/app.bin --emit-schema
```

This writes `app.bin` and `app.state.schema.json`.

Plain bindings accept JSON strings, numbers, or booleans. Dotted paths and
loops infer nested objects and arrays. Values used only by conditions are
optional because missing condition state evaluates false.

Broad values may include `x-webui.preferredType` as a non-validating hint for
host-language type generation. Integer literals infer `integer`, and known
types propagate across path-to-path equality.

Routed applications produce one named schema per complete route chain. The
`x-webui-routes` mapping connects each route pattern to its state definition,
allowing build-time tools to generate route-specific types without adding
runtime validation overhead.

## Learn More

- [Signals](/guide/concepts/directives/signals) - Template binding syntax
- [For loops](/guide/concepts/directives/for) - Iterating over collections
- [If conditions](/guide/concepts/directives/if) - Conditional rendering
- [Handlers](/guide/integrations/) - Passing state to the renderer