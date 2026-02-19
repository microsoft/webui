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

## Protocol Output

When the parser processes a `<for>` directive, it generates a `WebUIFragmentFor` in the protocol with the following properties:
- `item`: The name of the loop variable
- `collection`: The name of the array to iterate over
- `fragmentId`: A reference to the content template for each iteration
