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

The loop variable (e.g., `item` in the example above) is available within the loop body and can be used with the <code v-pre>{{}}</code> signal syntax:

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

When an `@observable` array changes on the client, the `<for>` block uses
cursor-based reconciliation (similar to Preact) that minimizes DOM mutations:

| Operation | DOM Cost | Description |
|-----------|----------|-------------|
| Append | O(1) | Only the new node is inserted |
| Prepend | O(1) | Only the new node is inserted; existing nodes untouched |
| Remove | O(1) | Only the removed node is detached |
| Reorder | O(moved) | Only nodes that changed position are moved |

### Keyed Reconciliation

The first attribute in the repeat block template serves as the key:

```html
<for each="item in items">
  <todo-item id="{{item.id}}" title="{{item.title}}"></todo-item>
</for>
```

Here, `item.id` (from the `id` attribute) is used as the key. Keyed
reconciliation correctly handles reordering, insertion, and removal.

### Unkeyed Reconciliation

Without a key attribute, items are matched by index. This is suitable for
append-only lists but not for reordering or arbitrary insertion.

### Key Collision Warning

Repeat item keys must be unique across the entire collection. If the server
generates items 1–1000 and the client starts at id=1000, keys collide and
the diff recreates all items - causing a visible flash.

**Best practice:** Use server-provided `nextId` in the state, UUIDs, or
start client IDs at `max(server_ids) + 1`.
