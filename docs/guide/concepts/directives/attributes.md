---
title: "Attribute and property bindings"
description: "Bind text attributes, conditional boolean attributes, and structured child properties from template state."
---

# Attribute Directives

Use an ordinary binding for an attribute value, `?` for conditional presence,
or `:` to pass a value to a child DOM property.

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

A whole-binding attribute is omitted when its value is missing or `null`.
Empty strings, `0`, and `false` remain attribute values; use `?` when presence
should depend on truthiness. Mixed attributes keep their static text.

## Boolean Attributes (`?` prefix)

Prefix an attribute name with `?` to make it conditional. The attribute is only rendered if the bound expression evaluates to a truthy value:

```html
<button ?disabled={{isDisabled}}>Submit</button>
<input type="checkbox" ?checked={{isSelected}} />
```

The value **must** be a pure binding: either a single path or an expression
accepted by `<if condition="...">`. Plain text and mixed static/dynamic values
cause the attribute to be silently dropped.

### Prefer Expressions Over Mirror Observables

Derive presence from existing state rather than maintaining parallel
`isFirst`, `isLast`, or `isSelected` observables:

```html
<!-- ✅ Good: derive from existing state -->
<button ?disabled="{{currentIndex == 0}}">Prev</button>
<button ?disabled="{{currentIndex == lastIndex}}">Next</button>
<option ?selected="{{item.id == selectedId}}">{{item.name}}</option>

<!-- ❌ Avoid: mirror observables that duplicate derivable state -->
<!-- @observable isPrevDisabled, isNextDisabled, isItemSelected — every
     update path now has to remember to recompute them. -->
<button ?disabled="{{isPrevDisabled}}">Prev</button>
```

The expression updates when a referenced value changes; a separate flag would
need to be kept in sync.

Expressions compare a dotted path against a literal or another dotted path. There is **no arithmetic**: `{{currentIndex == items.length - 1}}` is read as a comparison against a state key literally named `items.length - 1`, which never resolves, so the attribute is silently never rendered. Supply a precomputed `lastIndex` instead. A bare `.length` is fine on its own (`{{items.length > 0}}`).

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

Prefix a name with `:` to write a child DOM property, including structured
values that do not fit in a string attribute:

```html
<my-component :config="{{settings}}"></my-component>
<my-component :prop1="{{val1}}" :prop2="{{val2}}"></my-component>
```

For client-created trees, initial property bindings are applied before the
child's `connectedCallback`. Later parent updates flow through the same binding.
See [Property Bindings](/guide/concepts/interactivity#property-bindings).

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
