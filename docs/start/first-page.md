---
description: Render your first WebUI page from an HTML template and a JSON data file.
---

<span id="hello-world-tutorial"></span>

# Your first page

Create an HTML template and give it data. The result is a page that displays
**Hello, Ada!**, even with browser JavaScript disabled.

## Project setup

Use the `hello-webui` project from [Installation](/start/installation#install-the-cli).
That step creates `package.json`, installs the CLI, and checks that it runs.
If you already completed it, do not create another project.

Work in that directory for every command below. Create `src` and `data`
directories in your editor:

```text
hello-webui/
  package.json
  src/
    index.html
  data/
    state.json
```

## Creating the template

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
  </main>
</body>
</html>
```

The braces mark [text bindings](/learn/directives/signals).
For example, <code v-pre>{{name}}</code> takes the `name` value from your data.
WebUI escapes text values before writing them into the HTML.

## Creating the state

**`data/state.json`**

```json
{
  "title": "My first WebUI page",
  "name": "Ada",
  "message": "This HTML was rendered on the server."
}
```

This file is the [state](/learn/state-management/) for the local
preview: the values needed to render the page. It is separate from the template
so the same HTML can display different data.

## Previewing with `webui serve`

```bash
npm run dev
```

Use `pnpm run dev`, `yarn run dev`, or `bun run dev` if you installed with
that manager. Open **http://127.0.0.1:3000/**:

```text
Hello, Ada!
This HTML was rendered on the server.
```

The CLI reads your template and state, then serves the rendered HTML. There is
no application script or browser runtime to install for this page.

To try another value, stop the server with **Ctrl+C**, change `name` to
`"Grace"` in `data/state.json`, and run `npm run dev` again. The heading becomes
**Hello, Grace!**. Set it back to `"Ada"` before the next lesson.

For automatic rebuilds while editing, use `npm run dev -- --watch`. If port 3000
is occupied, use `npm run dev -- --port 3001`. See the
[serve command](/reference/cli/#webui-serve) for these development options.

<span id="what-weve-learned"></span>
<span id="next-steps"></span>

## Next step

[Add a component](/start/first-component) to put a reusable part of the page
in its own HTML and CSS files.

<span id="adding-interactivity"></span>
<span id="bundle-the-browser-entry"></span>
<span id="if-something-is-missing"></span>

The [Add interaction](/start/add-interactivity) lesson contains the working
counter, browser build setup, and troubleshooting.

<span id="building-for-production"></span>
<span id="rendering-with-rust"></span>

For deployment, read [Production builds](/learn/how-it-works#production-builds)
and choose a [host integration](/build/integrations/).
