# Attribute Directives

WebUI provides several ways to bind dynamic data to HTML attributes. When an attribute value contains handlebars expressions, the value is resolved from state at render time.

## Simple Dynamic Attributes

Use <code v-pre>{{}}</code> inside an attribute value to bind it to a state value:

```html
<a href="{{url}}">{{linkText}}</a>
<img src="{{imageUrl}}" alt="{{imageAlt}}" />
```

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

The value **must** be a pure handlebars expression (e.g., <code v-pre>{{signalName}}</code>). If the value contains plain text or mixed content, the attribute is silently dropped.

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

## Mixed Attributes

When an attribute value contains a mix of static text and dynamic expressions, each part is resolved independently:

```html
<input value="hello {{world}}" />
<use href="#icon-{{iconName}}" />
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

You can mix static, dynamic, boolean, and complex attributes on the same element:

```html
<my-component
  id="comp"
  :config="{{settings}}"
  ?enabled="{{isEnabled}}"
>
</my-component>
```

Static attributes (like `id="comp"`) are passed through as-is. Dynamic, boolean, and complex attributes are resolved from state at render time.
