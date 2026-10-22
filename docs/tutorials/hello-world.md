---
description: Build a scriptless WebUI page with the npm CLI, then bundle and hydrate a working counter.
---

# Hello World Tutorial

Start with a page rendered from HTML and JSON, with no browser JavaScript. Then
add a counter to learn where the browser runtime fits.

You need a current Node.js LTS release, npm, and a modern browser. The npm
package installs a native CLI; you do not need Rust or a WebUI source checkout.
See [installation](/guide/installation) if your platform needs extra setup.

## Project Setup

Create an empty directory and work inside it for **every command below**:

```bash
mkdir hello-webui
cd hello-webui
```

Create this package file, then install the compiler locally.

**`package.json`**

```json
{
  "name": "hello-webui",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "webui serve ./src --state ./data/state.json",
    "build": "webui build ./src --out ./dist"
  }
}
```

```bash
npm install --save-dev @microsoft/webui
```

Keep optional dependencies enabled: they carry the native binary for your
platform. npm scripts resolve the locally installed `webui` command.

Create the `src` and `data` directories in your editor. Your starting files are:

```text
hello-webui/
  package.json
  src/
    index.html
  data/
    state.json
```

## Creating the Template

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
    <if condition="showMessage">
      <p>{{message}}</p>
    </if>
    <ul>
      <for each="detail in details">
        <li>{{detail.label}}: {{detail.value}}</li>
      </for>
    </ul>
  </main>
</body>
</html>
```

The bindings look up data. `<if>` controls a region and `<for>` repeats it.
Text is escaped by default; this page does not need raw HTML bindings.
There is no stylesheet or script to create yet.

## Creating the State

**`data/state.json`**

```json
{
  "title": "My first WebUI page",
  "name": "Ada",
  "showMessage": true,
  "message": "This HTML was rendered on the server.",
  "details": [
    { "label": "Template", "value": "HTML" },
    { "label": "Data", "value": "JSON" }
  ]
}
```

This file supplies the initial render. A production backend would prepare this
data per request. Keep credentials and private tokens out of render state.

## Previewing with `webui serve`

```bash
npm run dev
```

Open **http://127.0.0.1:3000/**. You should see:

```text
Hello, Ada!
This HTML was rendered on the server.
• Template: HTML
• Data: JSON
```

The CLI compiles `src/index.html` and renders it with `data/state.json`.
The content is present even with browser JavaScript disabled.

Stop the server with **Ctrl+C**. To watch template and state edits while
developing, run `npm run dev -- --watch`. Watch mode adds a development
live-reload script; that script is not an application runtime requirement.
If port 3000 is occupied, use `npm run dev -- --port 3001`.

## Building for Production

```bash
npm run build
```

This creates **`dist/protocol.bin`**. It is a server artifact, not an HTML file
to open in the browser and not a static-site export. A production
[host integration](/guide/integrations) loads it once and renders it with
request data. Serve any generated CSS and your application's browser assets
from the paths used by the page.

## Adding Interactivity

Keep the same project. Stop the preview server before changing these files.
Install the browser runtime and a bundler:

```bash
npm install @microsoft/webui-framework
npm install --save-dev esbuild
```

Keep WebUI packages on matching releases. The compiler discovers the component
template; esbuild separately turns its TypeScript into browser JavaScript.

Create these files:

**`src/my-counter/my-counter.html`**

```html
<p>{{label}}: {{count}}</p>
<button type="button" @click="{increment()}">Increment</button>
```

**`src/my-counter/my-counter.css`**

```css
:host {
  display: block;
  margin-block: 1rem;
  font-family: system-ui, sans-serif;
}

button {
  font: inherit;
  padding: 0.5rem 1rem;
}
```

The unwrapped component uses Shadow DOM by default, so `:host` styles the
`<my-counter>` host and the button rule stays inside its shadow root.

**`src/my-counter/my-counter.ts`**

```typescript
// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr, observable } from '@microsoft/webui-framework';

export class MyCounter extends WebUIElement {
  @attr label = '';
  @observable count = 0;

  increment(): void {
    this.count += 1;
  }
}

MyCounter.define('my-counter');
```

`@attr` exposes an HTML attribute; `@observable` connects a value changed by
TypeScript to its template binding. The `define()` call registers the element.

**`src/index.ts`**

```typescript
// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import './my-counter/my-counter.js';
```

The `.js` import is intentional: esbuild resolves it to the TypeScript source.

Replace the entry template with this complete file:

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
    <if condition="showMessage">
      <p>{{message}}</p>
    </if>
    <ul>
      <for each="detail in details">
        <li>{{detail.label}}: {{detail.value}}</li>
      </for>
    </ul>
    <my-counter label="Clicks"></my-counter>
  </main>
  <script type="module" src="/index.js"></script>
</body>
</html>
```

Replace the state file to include the counter's initial value:

**`data/state.json`**

```json
{
  "title": "My first WebUI page",
  "name": "Ada",
  "showMessage": true,
  "message": "This HTML was rendered on the server.",
  "details": [
    { "label": "Template", "value": "HTML" },
    { "label": "Data", "value": "JSON" }
  ],
  "count": 0
}
```

The server does not execute the TypeScript class. The JSON value produces
`Clicks: 0` in the initial HTML; the field initializer alone would not do that.

### Bundle the browser entry

**`tsconfig.json`**

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "experimentalDecorators": true,
    "useDefineForClassFields": false
  }
}
```

These decorator settings are required by WebUI's component decorators.
This setup uses esbuild to transform TypeScript, not to type-check it.

**`build-client.mjs`**

```javascript
// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { build } from 'esbuild';

await build({
  entryPoints: ['src/index.ts'],
  outfile: 'dist/index.js',
  bundle: true,
  format: 'esm',
  target: 'es2022',
  tsconfig: 'tsconfig.json',
  define: { __WEBUI_DEV__: 'true' },
});
```

In `package.json`, replace **only the `scripts` object** with the one below.
Keep the dependencies and other fields npm already added.

**`package.json`**

```json
{
  "scripts": {
    "build:client": "node build-client.mjs",
    "build:protocol": "webui build ./src --out ./dist --plugin webui",
    "build": "npm run build:client && npm run build:protocol",
    "dev": "webui serve ./src --state ./data/state.json --plugin webui --servedir ./dist"
  }
}
```

```bash
npm run build
npm run dev
```

`dist/index.js` contains the counter and browser runtime. The WebUI build emits
`dist/protocol.bin` and the paired component CSS. `--servedir ./dist` makes the
browser bundle available at `/index.js`; the preview server handles component
CSS. Open **http://127.0.0.1:3000/** and click **Increment**: `Clicks: 0` becomes
`Clicks: 1`.

For this small app, no router, projection manifest, or manual hydration call is
needed. Without a projection manifest, hydration preserves full state. Keep
this data small and public; see [state projection](/guide/concepts/hydration#build-time-state-projection)
as the app grows.

When changing TypeScript, run `npm run build:client` and refresh the page.
`webui serve --watch` watches templates and data but does not bundle TypeScript.
For a release bundle, set `__WEBUI_DEV__` to `'false'` and enable esbuild's
`minify` option after testing.

### If something is missing

| Symptom | Check |
| --- | --- |
| CLI cannot find the app or state | Run commands from `hello-webui`; the files are `src/index.html` and `data/state.json`. |
| Page loads but the button does nothing | Build the client, use `--plugin webui`, and confirm `/index.js` returns JavaScript rather than a 404. |
| Decorator compilation fails or values do not update | Keep both settings in `tsconfig.json` and make sure the bundle uses that file. |
| Initial count is blank | Add `count` to server state. A class initializer is not SSR data. |

## Rendering with Rust

Rust is an optional production host, not a prerequisite for this tutorial.
The [Rust integration guide](/guide/integrations/rust) covers loading
`protocol.bin` and rendering requests. Other
[host integrations](/guide/integrations) use the same template and state model.

## What We've Learned

The server renders the first page from HTML and JSON. The browser runtime is
added explicitly, then connects a component's events and observable state to
that existing HTML. Updating browser state does not persist it to a backend.

## Next Steps

- Build the [Todo app](/tutorials/todo-app) for lists and child-to-parent events.
- Use the [React bridge](/guide/concepts/react-comparison) to translate familiar
  component patterns.
- Read [How It Works](/guide/concepts/how-it-works) for the build/server/browser
  boundaries.
