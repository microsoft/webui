# WebUI Framework E2E Test Fixtures

Each fixture is a minimal WebUI app that exercises a specific framework feature.

## Fixture format

```
fixtures/<name>/
  src/
    index.html                 Page template (uses the component)
    <tag-name>/
      <tag-name>.html          Component template (real WebUI syntax)
      <tag-name>.css           Component CSS (optional; needs a `css` mode in webui.config.json)
  state.json                   Initial render state (all bound properties)
  element.ts                   Component class (extends WebUIElement)
  <name>.spec.ts               Playwright tests
  webui.config.json            Build options override (optional, e.g. {"css":"module"})
```

## How it works

The test server (`tests/server.ts`) uses `fixture-render.ts` to:

1. **Discover** fixture dirs that have `src/index.html`
2. **Build** each via `@microsoft/webui` `build()` → compiles templates to protocol
3. **Render** via `render()` → produces SSR HTML with hydration markers, template metadata, condition closures, and inventory
4. **Inject** the `<script>` tag for the bundled `element.ts`
5. **Serve** the result at `/<name>/fixture.html`

Static files (JS bundles, CSS) are served from the fixtures root as-is.

## Creating a new fixture

1. Create `fixtures/<name>/src/index.html` with your page template
2. Create `fixtures/<name>/src/<tag>/<tag>.html` for each component
3. Create `fixtures/<name>/state.json` with initial property values
4. Create `fixtures/<name>/element.ts` with the component class — **no** `registerCompiledTemplate`
5. Create `fixtures/<name>/<name>.spec.ts` with Playwright tests
6. Run `pnpm test` to verify

### Template syntax

```html
<!-- Unwrapped component template (Light DOM) -->
<span>{{propertyName}}</span>
<button @click="{handler()}">Click</button>
<if condition="show"><p>Visible</p></if>
<for each="item in items"><li>{{item.name}}</li></for>
```

### State

`state.json` must include **all** properties used in template bindings with their
initial values. These are used for SSR rendering:

```json
{ "greeting": "Hello", "count": 0, "items": [{ "name": "Alpha" }] }
```

## Dynamic children pattern

Components only created via `document.createElement()` (not in any template) won't
have their template metadata included in the pipeline output because the handler only
emits templates for **reachable** components.

**Fix:** Add the child to the page template inside a false `<if>` block:

```html
<body>
  <my-host></my-host>
  <if condition="showChild"><my-child></my-child></if>
</body>
```

With `state.json`: `{ "showChild": false }`. This makes the child reachable (so its
template metadata is emitted) without rendering it during SSR.

## DOM ownership fixtures

The real build pipeline produces Light DOM for unwrapped templates. Use ordinary
source templates to test the production Light path. To test a Shadow component,
make a sole top-level `<template shadowrootmode="open">` its complete template.

The `light-dom` fixture exercises the unwrapped real-pipeline path and includes a
component-level Shadow opt-in. Use manual template registration or hand-written
fixture HTML only for tests that intentionally bypass the compiler.

## Per-fixture build config

Create `webui.config.json` to override build options:

```json
{ "css": "module" }
```

Supported keys: `css` (`"link"` | `"style"` | `"module"`), `dom`
(`"shadow"` | `"light"`; Shadow is the default), and `script` (`"module"`;
classic scripts are the default).
