---
description: Add a reusable HTML and CSS component to your WebUI page without a TypeScript class.
---

# Add a component

Put a count in its own component. This lesson adds a **Clicks: 0** display to
the page; the next lesson adds its button.

Continue in the `hello-webui` project from
[Your first page](/start/first-page). Stop the preview server with
**Ctrl+C** before changing these files.

## Define the component

Create a `src/my-counter` directory with two files.

**`src/my-counter/my-counter.html`**

```html
<p>{{label}}: {{count}}</p>
```

**`src/my-counter/my-counter.css`**

```css
:host {
  display: block;
  margin-block: 1rem;
  font-family: system-ui, sans-serif;
}
```

The filename defines the `<my-counter>` tag. WebUI pairs it with the CSS file
of the same name. The hyphen is required in a component name.

By default, the component renders inside a Shadow DOM boundary. `:host` styles
the `<my-counter>` element itself. The [component guide](/learn/components/)
explains file naming and the available DOM modes.

## Use it in the page

Replace the entry template with this file:

**`src/index.html`**

```html
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>{{title}}</title>
</head>
<body>
  <main>
    <h1>Hello, {{name}}!</h1>
    <p>{{message}}</p>
    <my-counter label="Clicks"></my-counter>
  </main>
</body>
</html>
```

`label="Clicks"` supplies the component's label. Add `count` to the existing
state file for its initial value:

**`data/state.json`**

```json
{
  "title": "My first WebUI page",
  "name": "Ada",
  "message": "This HTML was rendered on the server.",
  "count": 0
}
```

The label is an [attribute](/learn/directives/attributes); the count
comes from the page's [state](/learn/state-management/).

## Enable component output

Replace only the `scripts` object in your existing `package.json`. Keep the
dependencies, approvals, and other fields from installation.

**`package.json`**

```json
{
  "scripts": {
    "dev": "webui serve ./src --state ./data/state.json --plugin webui",
    "build": "webui build ./src --out ./dist --plugin webui"
  }
}
```

`--plugin webui` enables WebUI's component output. It does not require an
authored browser entry or a TypeScript class.

## Run it

```bash
npm run dev
```

Open **http://127.0.0.1:3000/**. Below the greeting, the page displays
**Clicks: 0**. It still renders with browser JavaScript disabled.

The component only displays data, so it has no `.ts` file. Changing the data
and rendering again is enough to change its initial content.

## Next step

[Add interaction](/start/add-interactivity) to make the count change when a
button is clicked.
