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

Write the `each` expression directly, without binding braces.

## Recursive Loops

Give a loop an `id` to reuse its body. A self-closing `<for>` with the same `id`
renders that body for a different collection, including an item's children:

```html
<ul>
  <for id="tree-item" each="child in items">
    <li>
      <span>{{child.name}}</span>
      <if condition="child.children.length">
        <ul>
          <for id="tree-item" each="child in child.children" />
        </ul>
      </if>
    </li>
  </for>
</ul>
```

Each level binds `child` to its own item. Returning from the nested loop restores
the parent's `child`. Missing or empty child arrays stop the nested repeat.
Use finite tree data and iterate over a smaller child collection at each level;
repeating the original collection indefinitely does not terminate.

An `id` is local to its entry or component file, so different files can both
define `tree-item`. Define its body once with a paired `<for>...</for>` and use
self-closing references elsewhere in that file. References may appear before
the definition. The defining loop also renders normally; it is not a hidden
template declaration. A second definition with children fails with
`duplicate-for-id`; use a self-closing reference instead.

A reference must use the definition's item name. For a definition using
`each="foo in bar"`, recurse with `each="foo in foo.children"`. The collection
`foo.children` is evaluated using the parent's `foo` first; each child then
becomes the new `foo` inside the shared body. Returning from the nested loop
restores the parent's binding. A reference reuses only the body, not the
definition's original collection expression.

IDs are static, non-empty names containing ASCII letters, digits, `_`, or `-`.
`id` is the only supported naming attribute; replace `template` with `id` on
definitions and references. The removed `template` spelling is a build error.
An unknown reference, duplicate definition, incompatible item variable, or
conflicting identifier fails the build with an actionable diagnostic.

Recursive repeats use the same positional or explicit-key reconciliation as
ordinary repeats. Put an optional `key` on the definition's first concrete
child; references reuse that key, with uniqueness checked within each sibling
collection.

Named recursion is supported in server-rendered templates and the native WebUI
client. FAST component builds reject named references with
`fast-named-for-unsupported`; use the WebUI plugin for interactive recursive
trees or ordinary nested loops in FAST components.

## Notes and Limitations

- The collection must be an array
- The item variable is only available within the loop body
- You can access nested properties of the item using dot notation
- A missing collection renders zero items; a present non-array collection is a rendering error
- **Unnamed empty `<for>` bodies** (with no children) are silently skipped.
  A paired named loop can define an empty body; a self-closing named loop is a reference.

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

The `key` attribute must be on the first concrete element inside `<for>`. If
that element is wrapped by one or more leading `<if>` directives, put `key` on
the concrete element inside the conditional. A nested `<for>` owns its own
child key. Putting `key` directly on `<if>`, `<for>`, or `<outlet>` fails with
`invalid-for-key`.

The value must be a single binding to the loop variable or a dot-separated
property path rooted at it. Calls, brackets, operators, static values, empty
paths, unrelated variables, and `key` on another regular element fail with the
same diagnostic.

`key` is compiler-only metadata: it does not render into SSR or browser-created
HTML and does not become a reactive attribute binding. `data-key` remains a
normal application-visible attribute and does not control repeat identity.

Key values must be unique strings or finite numbers. If an update produces a
duplicate or invalid value, WebUI warns once and safely uses positional
reconciliation for that update. A later valid update re-establishes keyed
identity.
