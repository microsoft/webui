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

Bindings resolve state paths using dot notation. Supported patterns:

| Pattern | Example | Resolves to |
|---------|---------|-------------|
| Simple property | `title` | `"My App"` |
| Nested property | `user.profile.avatar` | `"/img/alice.png"` |
| Array length | `items.length` | `2` |
| String length | `title.length` | `6` |

Numeric array indexes such as `items.0.label` are not supported. Bind array
items with [`<for>`](/guide/concepts/directives/for) instead.

Array `.length` counts elements. String `.length` counts **UTF-8 bytes**, not
characters or JavaScript UTF-16 code units: `"é"` has length `2` and `"😀"` has
length `4`. This rule is the same on the server and in browser template
bindings. If the UI needs a character or grapheme count, supply that count as a
separate state value.

The synthetic `.length` ends an array or string path; `items.length.more`
does not resolve. A real object property named `length` follows ordinary
property lookup.

Paths are resolved at render time. If a path doesn't exist in the state, text
and attribute bindings render as empty. In conditions, a missing identifier is
a falsy operand: `<if condition="path">` does not render, while
`<if condition="!path">` does render. No error is reported for a missing
identifier path in those bindings. A
[`<render>` input](/guide/concepts/directives/fragment#inputs-and-scope) is
different: its `scope` path must resolve, or rendering fails with an actionable
error. An existing `null`, `false`, `0`, empty string, object, or array is a
valid input, not a missing path.

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
- **Owner state** remains the fallback for a missing loop-item member. Fragment
  aliases differ: they own their entire root name and never fall back for
  missing children.
- **Nested loops**: outer loop items remain accessible by their monikers (for
  example, <code v-pre>{{category.name}}</code> inside a `product` loop), unless
  an inner loop uses the same moniker.
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

## State in Local Fragments

A [`<render>` call](/guide/concepts/directives/fragment) selects its input in
the caller's scope and makes it available under the `as` alias. Inside the
fragment, its own loop variables take precedence over that alias, which takes
precedence over the owning component's state and props.

Caller loop variables and caller fragment aliases are hidden, including for a
parameterless call. The alias owns its whole root name: missing children never
fall back to an owner value with the same root.

```html
<render fragment="details" scope="{{selectedPerson}}" as="person"></render>
```

For this call, the fragment reads `person.name` from `selectedPerson.name`.
It can still read other owner state such as `title`, but cannot read a caller's
loop variable unless that value was explicitly passed.

During hydration, state omitted from the browser bootstrap is unavailable,
not proof that the server input was missing. WebUI preserves the trusted SSR
content until that state is supplied. Once a root is known, a missing requested
child is a real missing-input error. See
[Hydration](/guide/concepts/hydration#local-fragment-hydration).

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

Unlike client-side frameworks that fetch data on mount, WebUI renders in a
single pass. The state object should contain everything the template needs for
first render. Missing values render as empty output for text and attribute
bindings. A missing condition identifier is falsy before logical operators are
applied, so its positive branch is hidden and its negated branch is shown.

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
request state complete for every SSR binding, condition, loop, and fragment
input. A validated
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

## Learn More

- [Signals](/guide/concepts/directives/signals) - Template binding syntax
- [For loops](/guide/concepts/directives/for) - Iterating over collections
- [Local fragments](/guide/concepts/directives/fragment) - Reusable markup with explicit inputs
- [If conditions](/guide/concepts/directives/if) - Conditional rendering
- [Handlers](/guide/integrations/) - Passing state to the renderer