---
title: "Text and raw HTML bindings"
description: "Read state through dotted paths, escape text by default, and reserve raw bindings and document injection for trusted HTML."
---

<a id="signal-directives"></a>

# Text Bindings

Insert state with <code v-pre>{{path}}</code>. Use triple braces only for
trusted HTML. These bindings read values; they do not evaluate arbitrary
JavaScript, call functions, or perform arithmetic.

## Escaped Signals: `{{}}`

The double curly braces syntax (`{{}}`) inserts a value from the state object with HTML escaping applied:

```html
<p>Hello, {{user.name}}!</p>
```


HTML characters in the value are escaped so they display as text rather than
markup. This is not URL validation: validate URL-bearing values separately.

### Example
State:
```json
{
  "user": {
    "name": "John <script>alert('XSS')</script>"
  }
}
```

Output:
```html
<p>Hello, John &lt;script&gt;alert(&#x27;XSS&#x27;)&lt;&#x2F;script&gt;!</p>
```

## Raw Signals: `{{{}}}`

The triple curly braces syntax (``{{{}}}``) inserts a value from the state object without HTML escaping:

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

Use `.length` to read a collection size:

- `array.length` - Returns the number of items in the array

Example:
```html
<p>There are {{items.length}} items in the list.</p>
```

## Body Signals

Hosts can add trusted HTML at document boundaries through the reserved
`$webui` state object. Authors do not write `body_start` or `body_end` bindings
in their templates.

### Example

State:
```json
{
  "$webui": {
    "bodyStart": "<!-- host-owned content -->",
    "bodyEnd": "<script src=\"/livereload.js\"></script>"
  }
}
```

Output:
```html
<body>
  <!-- host-owned content -->
  <app-shell></app-shell>
  <script src="/livereload.js"></script>
</body>
```

`headEnd` is also supported. These strings are emitted without escaping, and
the reserved object is excluded from browser state. Never populate it from
untrusted request data. See the [host integration reference](/build/integrations/).
