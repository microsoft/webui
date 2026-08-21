# Components

Components are the building blocks of WebUI applications. They leverage the native [Web Components](https://developer.mozilla.org/en-US/docs/Web/API/Web_components) standard to provide encapsulated, reusable UI elements with efficient server-side rendering.

## Component Discovery

WebUI uses a component discovery system that automatically scans and registers components at build time:

1. The framework scans specified directories for component files
2. It identifies HTML files with hyphenated names as components
3. It associates matching CSS and JS files with their components
4. The discovered components are compiled into the WebUI protocol

### Component File Structure

```
my-component.html  # Required - component template
my-component.css   # Optional - component styles
my-component.js    # Optional - authored client behavior
```

Components must follow these naming conventions:

- **Hyphen required**: All component names must contain at least one hyphen (e.g., `user-card`, `nav-menu`, `data-table`)
- **File name = component name**: The HTML file name determines the component's tag name

An HTML-only component still receives compiled browser template metadata, but
it contributes no initial browser state. When the framework runtime is loaded,
it can activate the component for browser-applied state or soft navigation. Add
the JavaScript or TypeScript file only for authored events, lifecycle code,
decorators, or imperative APIs.

### The `<template>` Tag

Most components write only their content. Shadow is the default fallback for
that unwrapped content; build with `--dom light` to render it directly in the
host:

```html
<!-- user-card.html -->
<img src="{{avatar}}" alt="{{name}}" />
<h3>{{name}}</h3>
<p>{{email}}</p>
```

A sole bare top-level `<template>` is an explicit Light-mode wrapper and is
unwrapped even when the build fallback is Shadow. Templates with attributes or
policy directives do not select a mode; use the `shadowrootmode` attribute for
an explicit Shadow root.

In a Light build, use a sole top-level
`<template shadowrootmode="open">` when a component must remain Shadow for a
native `<slot>`, native encapsulation, or root events on the host element:

```html
<!-- task-list.html -->
<template shadowrootmode="open"
  @task-complete="{onTaskComplete(e)}"
  @task-delete="{onTaskDelete(e)}"
>
  <for each="task in tasks">
    <task-item id="{{task.id}}" title="{{task.title}}"></task-item>
  </for>
</template>
```

The wrapper must contain the complete component. Closed roots, invalid values
or placement, additional top-level content, and `<slot>` in an unwrapped
component fail the build. The compiler never generates this wrapper.

Component templates must use browser-valid HTML nesting. WebUI recognizes native
void tags case-insensitively and accounts for the `<colgroup>` and `<tbody>` that
browsers imply around direct `<col>` and `<tr>` runs. If an `<if>` or `<for>`
controls table columns or rows, write the `<colgroup>` or `<tbody>` explicitly
so its SSR hydration markers share one parser context.

## How Components Work

When WebUI discovers components:

1. **Build Time**:
   - The component's HTML is parsed and tokenized
   - Any directives (`<if>`, `<for>`, etc.) and signals (`{{}}`) are processed
   - The component's CSS is analyzed and included in the protocol
   - A unique `fragmentId` is assigned to each component

2. **Runtime**:
   - The server-side handler renders components based on state
   - Unwrapped components follow the build's Shadow/Light fallback
   - Components with a valid sole open wrapper always output Declarative Shadow DOM
   - Dynamic content is injected according to the protocol

## Component Organization

For larger applications, we recommend organizing components following an Atomic Design-inspired structure:

```
app/
├── src/
│   ├── components/
│   │   ├── atoms/
│   │   │   ├── button/
│   │   │   │   ├── button.html
│   │   │   │   └── button.css
│   │   │   ├── input/
│   │   │   └── icon/
│   │   ├── molecules/
│   │   │   ├── search-box/
│   │   │   ├── notification/
│   │   │   └── menu-item/
│   │   └── organisms/
│   │       ├── navigation/
│   │       ├── user-profile/
│   │       └── product-card/
│   ├── layouts/
│   │   ├── default-layout.html
│   │   └── dashboard-layout.html
│   ├── views/
│   │   ├── home/
│   │   ├── products/
│   │   └── settings/
│   └── app.html
├── public/
└── config.json
```

### Component Levels

- **Atoms**: Basic building blocks (buttons, inputs, icons)
- **Molecules**: Simple combinations of atoms (search boxes, menu items)
- **Organisms**: Complex UI sections composed of molecules and atoms
- **Layouts**: Page structures that components fit into
- **Views**: Complete page templates composed of various components

## Using Components

Once defined, components can be used throughout your application, in this 
example we have `profile-page.html`, `user-card.html`, and `admin-controls.html`:

```html
<!-- profile-page.html -->
<div class="profile-container">
  <h1>User Profile</h1>
  <user-card></user-card>
  
  <if condition="isAdmin">
    <admin-controls></admin-controls>
  </if>
</div>
```

## Component TypeScript Classes

Interactive components have a TypeScript class that defines their behavior.
The class extends `WebUIElement` from `@microsoft/webui-framework`:

```typescript
import { WebUIElement, attr, observable } from '@microsoft/webui-framework';

export class UserCard extends WebUIElement {
  @attr name = '';
  @attr email = '';
  @observable isExpanded = false;

  toggle(): void {
    this.isExpanded = !this.isExpanded;
  }
}

UserCard.define('user-card');
```

The TypeScript file lives alongside the HTML and CSS:

```
user-card/
├── user-card.html   ← Template (declarative)
├── user-card.css    ← Styles (scoped at build time)
└── user-card.ts     ← Behavior (TypeScript class)
```

### Separation of Concerns

WebUI intentionally keeps HTML, CSS, and TypeScript in separate files:

- **HTML** defines structure and data bindings (`{{expr}}`, `<if>`, `<for>`)
- **CSS** defines visual presentation. WebUI scopes Light CSS and preserves
  native Shadow scoping for Shadow components
- **TypeScript** defines interactive behavior (event handlers, state mutations)

There is no JSX, no CSS-in-JS, and no template literals. This separation
is a performance decision: the HTML template is compiled to binary at build
time, and only the TypeScript ships to the browser for interactive components.

For the full interactivity guide, see [Interactivity](/guide/concepts/interactivity).

## External Component Sources

In addition to discovering components in your app directory, WebUI can load components from **npm packages** and **local paths** using the `--components` CLI flag.

### npm Packages

Components published as npm packages can be discovered automatically. The package must:

1. Be installed via npm, pnpm, or yarn (present in `node_modules/`)
2. Include a `package.json` with:
   - `exports["./template-webui.html"]` - the component's HTML template
   - `exports["./styles.css"]` - the component's CSS (optional)
   - `customElements` - path to a [Custom Elements Manifest](https://github.com/webcomponents/custom-elements-manifest) JSON file

The Custom Elements Manifest provides the component's tag name:

```json
{
  "schemaVersion": "1.0.0",
  "modules": [{
    "kind": "javascript-module",
    "declarations": [{
      "kind": "class",
      "name": "MyButton",
      "tagName": "my-button"
    }]
  }]
}
```

**Example `package.json`:**

```json
{
  "name": "@reactive-ui/button",
  "version": "1.0.0",
  "customElements": "./custom-elements.json",
  "exports": {
    "./template-webui.html": "./dist/template-webui.html",
    "./styles.css": "./dist/styles.css"
  }
}
```

**Scoped packages:** When you pass a bare scope like `@reactive-ui`, all sub-packages under `node_modules/@reactive-ui/` are discovered and each is checked for WebUI component exports.

Packages that also expose a root JavaScript entry (`exports["."]`, `main`,
`module`, or `browser`) are treated as authored custom-element packages. Packages
with only WebUI template/style exports are treated as HTML-only component
libraries; dynamic bindings render on the server and remain inactive until the
framework needs them.

### Local Paths

You can also point to directories outside your app folder:

```bash
webui build ./my-app --out ./dist --components ./shared/components
```

Local path discovery works identically to app directory scanning - HTML files with
hyphenated names are registered as components, matching CSS files are auto-paired,
and a sibling `.ts` or `.js` file marks that component as authored/interactive.

### Caching

npm package discovery results are cached at `~/.webui/cache/components/`. The cache invalidates automatically when a package's `package.json` content changes. Local path sources are always re-scanned.

See the [CLI Reference](/guide/cli/) for full `--components` usage.
