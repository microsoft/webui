# `<for>` Loop Directive

The `<for>` directive allows you to iterate over arrays and generate repeated content for each item.

## Basic Usage

```html
<for each="item in items">
  <div class="item">
    <h3>{{item.title}}</h3>
    <p>{{item.description}}</p>
  </div>
</for>
```

This will generate a div for each item in the `items` array, with the current item available as `item` within the loop.

## Using the Loop Variable

The loop variable (e.g., `item` in the example above) is available within the loop body and can be used with the `{{}}` signal syntax:

```html
<ul>
  <for each="person in people">
    <li>{{person.name}} ({{person.age}} years old)</li>
  </for>
</ul>
```

## Nested Loops

You can nest `<for>` directives to iterate over nested collections:

```html
<for each="category in categories">
  <h2>{{category.name}}</h2>
  <ul>
    <for each="product in category.products">
      <li>{{product.name}} - ${{product.price}}</li>
    </for>
  </ul>
</for>
```

## Condition Format

The `each` attribute must follow this format:

```
itemName in collectionName
```

Where:
- `itemName` is the name for the current item variable
- `collectionName` is the path to the array in the state object

## Notes and Limitations

- The collection must be an array
- The item variable is only available within the loop body
- You can access nested properties of the item using dot notation
- If the collection doesn't exist or isn't an array, an error will be raised during rendering
- **Empty `<for>` bodies** (with no children) are silently skipped - no output is generated

## Repeat Reconciliation

When an `@observable` array changes on the client, the `<for>` block reconciles
by array position by default. The existing block at index `i` receives the
current item at index `i`; new tail items are appended and excess tail blocks
are removed.

| Collection change | Runtime behavior |
|-------------------|------------------|
| Append | Reuse existing blocks and create the new tail |
| Truncate | Reuse the shared prefix and remove the excess tail |
| Replace or reorder | Rebind existing blocks at each position |

Duplicate values and duplicate attributes are safe. Dynamic attributes never
act as hidden keys, so changing their order cannot change reconciliation.

```html
<for each="tag in tags">
  <span class="{{tag.className}}">{{tag.label}}</span>
</for>
```

Because identity is positional, reordering items does not move their existing
subtrees by logical item. Browser-owned state such as focus or an uncontrolled
input value remains associated with its position.

### Explicit keys

Use an explicit key when reordered, prepended, or removed items must retain
their existing DOM and component-local state:

```html
<for each="item in items">
  <todo-row key="{{item.id}}" title="{{item.title}}"></todo-row>
</for>
```

For an array of unique strings or finite numbers, key the item itself:

```html
<for each="tag in tags">
  <span key="{{tag}}">{{tag}}</span>
</for>
```

The `key` attribute must be on the first child inside `<for>`. Its value must
be a single binding to the loop variable or a dot-separated property path
rooted at it. Calls, brackets, operators, static values, empty paths, and
unrelated variables fail the build with `invalid-for-key`. A `key` on another
regular element produces the same diagnostic. Attributes on directives remain
governed by each directive's own contract and do not provide repeat identity.

`key` is compiler-only metadata: it does not render into SSR or browser-created
HTML and does not become a reactive attribute binding. `data-key` remains a
normal application-visible attribute and does not control repeat identity.

Key values must be unique strings or finite numbers. If an update produces a
duplicate or invalid value, WebUI warns once and safely uses positional
reconciliation for that update. A later valid update re-establishes keyed
identity.
