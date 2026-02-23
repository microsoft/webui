# Attribute Directives

WebUI provides several ways to bind dynamic data to HTML attributes. When an attribute value contains handlebars expressions, the parser emits attribute fragments that are resolved at render time.

## Simple Dynamic Attributes

Use <code v-pre>{{}}</code> inside an attribute value to bind it to a state signal:

```html
<a href="{{url}}">{{linkText}}</a>
<img src="{{imageUrl}}" alt="{{imageAlt}}" />
```

When the entire attribute value is a single handlebars expression, WebUI produces a simple attribute binding:

```json
{ "type": "attribute", "name": "href", "value": "url" }
```

At render time, the signal name is resolved against the state to produce the final attribute value.

### Example

State:
```json
{
  "url": "https://example.com",
  "linkText": "Visit Example"
}
```

Output:
```html
<a href="https://example.com">Visit Example</a>
```

## Boolean Attributes (`?` prefix)

Prefix an attribute name with `?` to make it conditional. The attribute is only rendered if the bound expression evaluates to a truthy value:

```html
<button ?disabled={{isDisabled}}>Submit</button>
<input type="checkbox" ?checked={{isSelected}} />
```

The value **must** be a pure handlebars expression (e.g., `{{signalName}}`). If the value contains plain text or mixed content, the attribute is silently dropped.

### Dotted Paths

Boolean attributes support dotted path notation to access nested state:

```html
<div ?hidden={{layout.isPinned}}>Pinned content</div>
```

### Valid and Invalid Examples

```html
<!-- ✅ Valid: pure handlebars expression -->
<button ?disabled={{isDisabled}}>Click</button>

<!-- ✅ Valid: dotted path -->
<input ?checked={{form.agreed}} />

<!-- ❌ Invalid: plain value (attribute silently dropped) -->
<button ?disabled="true">Click</button>

<!-- ❌ Invalid: mixed content (attribute silently dropped) -->
<input ?checked="Hello {{name}}" />
```

### Example

State:
```json
{
  "isDisabled": true,
  "isSelected": false
}
```

Template:
```html
<button ?disabled={{isDisabled}}>Submit</button>
<input type="checkbox" ?checked={{isSelected}} />
```

Output:
```html
<button disabled>Submit</button>
<input type="checkbox" />
```

## Complex Attributes (`:` prefix)

Prefix an attribute name with `:` to create a complex binding. This is used for passing structured data to web components:

```html
<my-component :config="{{settings}}"></my-component>
<my-component :prop1="{{val1}}" :prop2="{{val2}}"></my-component>
```

Complex attributes preserve the `:` prefix in the attribute name and are marked with `complex: true` in the protocol:

```json
{ "type": "attribute", "name": ":config", "value": "settings", "complex": true }
```

## Mixed (Template) Attributes

When an attribute value contains a mix of static text and dynamic expressions, WebUI creates a template sub-stream:

```html
<input value="hello {{world}}" />
<use href="#icon-{{iconName}}" />
```

The parser splits the value into a separate fragment stream referenced by a template ID:

```json
{ "type": "attribute", "name": "value", "template": "attr-1" }
```

The sub-stream `attr-1` contains:
```json
[
  { "type": "raw", "value": "hello " },
  { "type": "signal", "value": "world" }
]
```

### Example

State:
```json
{
  "world": "WebUI"
}
```

Output:
```html
<input value="hello WebUI" />
```

## Combining Attribute Types

You can mix static attributes, dynamic attributes, boolean attributes, and complex attributes on the same element:

```html
<my-component
  id="comp"
  :config="{{settings}}"
  ?enabled="{{isEnabled}}"
>
</my-component>
```

Static attributes (like `id="comp"`) are passed through as raw HTML. Dynamic, boolean, and complex attributes each produce their own attribute fragment in the protocol.

## Protocol Output

The attribute fragment type supports all binding styles through a single flexible structure:

| Attribute Style | Protocol Fields |
|----------------|----------------|
| Simple dynamic | `name`, `value` |
| Boolean (`?`) | `name`, `conditionTree` |
| Complex (`:`) | `name`, `value`, `complex: true` |
| Mixed/template | `name`, `template` (references sub-stream) |
