# Attribute Directives

WebUI provides several ways to bind dynamic data to HTML attributes. When an attribute value contains handlebars expressions, the value is resolved from state at render time.

## Simple Dynamic Attributes

Use `{{}}` inside an attribute value to bind it to a state value:

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

The value **must** be a pure handlebars expression — either a single signal (e.g., `{{signalName}}`) or any expression that `<if condition="...">` accepts (comparisons, logical operators, dotted paths). If the value contains plain text or mixed static/dynamic content, the attribute is silently dropped.

### Prefer Expressions Over Mirror Observables

Boolean attributes accept the same expression syntax as `<if>`, so **derive boolean state from your existing observables** rather than creating parallel `isFirst` / `isLast` / `isSelected` observables that you have to keep in sync:

```html
<!-- ✅ Good: derive from existing state -->
<button ?disabled="{{currentIndex == 0}}">Prev</button>
<button ?disabled="{{currentIndex == items.length - 1}}">Next</button>
<option ?selected="{{item.id == selectedId}}">{{item.name}}</option>

<!-- ❌ Avoid: mirror observables that duplicate derivable state -->
<!-- @observable isPrevDisabled, isNextDisabled, isItemSelected — every
     update path now has to remember to recompute them. -->
<button ?disabled="{{isPrevDisabled}}">Prev</button>
```

Mirror observables are a common source of UI desync bugs: any code path that mutates `currentIndex` but forgets to update `isPrevDisabled` produces incorrect rendering. Expressions are evaluated automatically whenever any signal they reference changes.

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

<!-- ✅ Valid: comparison expression (preferred over mirror observables) -->
<button ?disabled={{currentIndex == 0}}>Prev</button>
<option ?selected={{item.id == selectedId}}>{{item.name}}</option>

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
