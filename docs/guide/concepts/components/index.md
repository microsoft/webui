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
my-component.js    # Optional - client-side behavior
```

Components must follow these naming conventions:

- **Hyphen required**: All component names must contain at least one hyphen (e.g., `user-card`, `nav-menu`, `data-table`)
- **File name = component name**: The HTML file name determines the component's tag name

### The `<template>` Tag

The `<template shadowrootmode="open">` wrapper is **optional** in component HTML files. The build tool auto-injects it when it is not present.

Most components omit it and write just the content:

```html
<!-- user-card.html -->
<img src="{{avatar}}" alt="{{name}}" />
<h3>{{name}}</h3>
<p>{{email}}</p>
```

Include it explicitly when you need **root host events** - event listeners on the shadow root that catch events bubbling up from children:

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

When you include the `<template>` tag, the framework uses yours instead of auto-injecting one.

## How Components Work

When WebUI discovers components:

1. **Build Time**:
   - The component's HTML is parsed and tokenized
   - Any directives (`<if>`, `<for>`, etc.) and signals (`{{}}`) are processed
   - The component's CSS is analyzed and included in the protocol
   - A unique `fragmentId` is assigned to each component

2. **Runtime**:
   - The server-side handler renders components based on state
   - Components are output as Declarative Shadow DOM elements
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
├── user-card.css    ← Styles (scoped via Shadow DOM)
└── user-card.ts     ← Behavior (TypeScript class)
```

### Separation of Concerns

WebUI intentionally keeps HTML, CSS, and TypeScript in separate files:

- **HTML** defines structure and data bindings (`{{expr}}`, `<if>`, `<for>`)
- **CSS** defines visual presentation (scoped via Shadow DOM)
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

### Local Paths

You can also point to directories outside your app folder:

```bash
webui build ./my-app --out ./dist --components ./shared/components
```

Local path discovery works identically to app directory scanning - HTML files with hyphenated names are registered as components, with matching CSS files auto-paired.

### Caching

npm package discovery results are cached at `~/.webui/cache/components/`. The cache invalidates automatically when a package's `package.json` content changes. Local path sources are always re-scanned.

See the [CLI Reference](/guide/cli/) for full `--components` usage.
