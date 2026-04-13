# WebUI Framework E2E Test Fixtures

Each fixture is a minimal WebUI app that exercises a specific framework feature.

## Fixture format

```
fixtures/<name>/
  src/
    index.html                 Page template (uses the component)
    <tag-name>/
      <tag-name>.html          Component template (real WebUI syntax)
      <tag-name>.css           Component CSS (optional, for css-module fixtures)
  state.json                   Initial render state (all bound properties)
  element.ts                   Component class (extends WebUIElement)
  <name>.spec.ts               Playwright tests
  webui.config.json            Build options override (optional, e.g. {"css":"module"})
```

## How it works

The test server (`tests/server.ts`) uses `fixture-render.ts` to:

1. **Discover** fixture dirs that have `src/index.html`
2. **Build** each via `@microsoft/webui` `build()` → compiles templates to protocol
3. **Render** via `render()` → produces SSR HTML with hydration markers, template IIFEs, and inventory
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
<!-- Component template (shadow DOM) -->
<template shadowrootmode="open">
  <span>{{propertyName}}</span>
  <button @click="{handler()}">Click</button>
  <if condition="show"><p>Visible</p></if>
  <for each="item in items"><li>{{item.name}}</li></for>
</template>
```

### State

`state.json` must include **all** properties used in template bindings with their
initial values. These are used for SSR rendering:

```json
{ "greeting": "Hello", "count": 0, "items": [{ "name": "Alpha" }] }
```

## Dynamic children pattern

Components only created via `document.createElement()` (not in any template) won't
have their template IIFEs included in the pipeline output because the handler only
emits templates for **reachable** components.

**Fix:** Add the child to the page template inside a false `<if>` block:

```html
<body>
  <my-host></my-host>
  <if condition="showChild"><my-child></my-child></if>
</body>
```

With `state.json`: `{ "showChild": false }`. This makes the child reachable (so its
template IIFE is emitted) without rendering it during SSR.

## Light-DOM fixtures

The pipeline always produces shadow DOM. The `light-dom` fixture uses manual
template registration (`registerCompiledTemplate`) and hand-written `fixture.html`
to keep the light-DOM hydration code path tested. Use this pattern for any test
that specifically targets light-DOM behavior.

## Per-fixture build config

Create `webui.config.json` to override build options:

```json
{ "css": "module" }
```

Supported keys: `css` (`"link"` | `"style"` | `"module"`).
