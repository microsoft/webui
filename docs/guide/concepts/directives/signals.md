# Signal Directives

WebUI provides two types of signals for inserting dynamic values into your templates: escaped signals and raw signals.

## Escaped Signals: <code v-pre>{{}}</code>

The double curly braces syntax (`{{}}`) inserts a value from the state object with HTML escaping applied:

```html
<p>Hello, {{user.name}}!</p>
```


This ensures that any special HTML characters in the value are properly escaped, preventing XSS attacks.

### Example
State:
```json
{
  "user": {
    "name": "John &lt;script&gt;alert('XSS')&lt;/script&gt;"
  }
}
```

Output:
```html
<p>Hello, John &lt;script&gt;alert('XSS')&lt;/script&gt;!</p>
```

## Raw Signals: <code v-pre>{{{}}}</code>

The triple curly braces syntax (<code v-pre>`{{{}}}`</code>) inserts a value from the state object without HTML escaping:

```html
<div>{{{rawHtmlContent}}}</div>
```

**Warning:** Only use raw signals with trusted content, as unescaped HTML can lead to security vulnerabilities.

### Example

State:
```json
{
  "rawHtmlContent": "<strong>Bold Text</strong> and <em>Emphasized Text</em>"
}
```

Output:
```html
<div><strong>Bold Text</strong> and <em>Emphasized Text</em></div>
```

## Accessing Nested Properties

You can access nested properties using dot notation:

```html
<span>{{user.address.city}}, {{user.address.country}}</span>
```

## Special Values

WebUI provides special values for arrays:

- `array.length` - Returns the number of items in the array

Example:
```html
<p>There are {{items.length}} items in the list.</p>
```

## Body Signals

WebUI automatically injects two special signals around `<body>` content:

- **`body_start`** — injected immediately after the `<body>` opening tag
- **`body_end`** — injected immediately before the `</body>` closing tag

These enable handlers to inject content at the start and end of the document body (e.g., hydration scripts, analytics tags).

### Example

Template:
```html
<body>
  <app-shell></app-shell>
</body>
```

Output (with handler-injected content):
```html
<body>
  <!-- handler can insert content here via body_start -->
  <app-shell></app-shell>
  <!-- handler can insert content here via body_end -->
</body>
```
