---
description: Add an event handler to your component, bundle its browser code, and update a server-rendered count.
---

# Add interaction

Make the count change when you click a button. The initial value still comes
from the server; the component handles clicks in the browser.

Continue in the `hello-webui` project from
[Add a component](/start/first-component). Stop the preview server with
**Ctrl+C** before changing the files below.

## Install the browser runtime

The CLI renders HTML. Interactive components also need the browser runtime
and a bundler for their TypeScript:

```bash
npm install @microsoft/webui-framework
npm install --save-dev esbuild
```

Keep WebUI packages on matching releases.
If your package manager reports blocked scripts for `esbuild`, review and
approve that package separately using the
[installation policies](/start/installation#installation-policies).

## Add a button and its handler

Replace the component template and stylesheet with these files:

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

`@attr` reads the `label` attribute. `@observable` connects `count` to the
template, so assigning a new value updates its text. `@click` calls
`increment()`. The `define()` call declares the component.

The [interactivity guide](/learn/interactivity) covers attributes,
observable state, and events in more detail.

## Load the component

Create a browser entry that imports the component:

**`src/index.ts`**

```typescript
// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { startRuntime } from '@microsoft/webui-framework/runtime.js';
import '@microsoft/webui-framework/styles.js';
import './my-counter/my-counter.js';

startRuntime();
```

The `.js` import is intentional: esbuild resolves it to the TypeScript source.
`startRuntime()` registers the imported component and starts hydration.

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
    <p>{{message}}</p>
    <my-counter label="Clicks"></my-counter>
  </main>
  <script type="module" src="/index.js"></script>
</body>
</html>
```

Keep `"count": 0` in `data/state.json` from the previous lesson. The server does
not execute the TypeScript class. The JSON value produces `Clicks: 0` in the
initial HTML; the class initializer alone would not do that.
See [state and data](/learn/state-management/) for the boundary between
initial server data and later browser updates.

## Bundle the browser entry

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
Keep your dependencies, script approvals, and other fields.

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

## Run the counter

```bash
npm run build
npm run dev
```

`dist/index.js` contains the counter and browser runtime. The WebUI build emits
`dist/protocol.bin` and the paired component CSS. `--servedir ./dist` makes the
browser bundle available at `/index.js`; the preview server handles component
CSS. Open **http://127.0.0.1:3000/** and click **Increment**: `Clicks: 0` becomes
`Clicks: 1`.

The browser connects the click handler to the rendered button. This is
[hydration](/build/hydration); it does not replace the first page with
a second render. Reloading resets the count to the value in `data/state.json`.

When changing TypeScript, run `npm run build:client` and refresh the page.
`webui serve --watch` watches templates and data but does not bundle TypeScript.
This is a development setup. [Production builds](/learn/how-it-works#production-builds)
cover release bundles and serving the compiled result.

## If something is missing

| Symptom | Check |
| --- | --- |
| CLI cannot find the app or state | Run commands from `hello-webui`; the files are `src/index.html` and `data/state.json`. |
| Page loads but the button does nothing | Build the client, use `--plugin webui`, and confirm `/index.js` returns JavaScript rather than a 404. |
| Decorator compilation fails or values do not update | Keep both settings in `tsconfig.json` and make sure the bundle uses that file. |
| Initial count is blank | Add `count` to server state. A class initializer is not SSR data. |

## Next step

[Build a Todo app](/start/todo-app) to work with lists and events between
components.
