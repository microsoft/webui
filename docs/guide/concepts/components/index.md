# Components

Components are the building blocks of WebUI applications. They leverage the native [Web Components](https://developer.mozilla.org/en-US/docs/Web/API/Web_components) standard to provide encapsulated, reusable UI elements with efficient server-side rendering.

## Component Discovery

WebUI automatically discovers components at build time:

1. WebUI resolves local or npm package roots
2. The selected plugin discovers component templates and styles
3. WebUI validates and compiles the components

The `webui` plugin identifies HTML files with hyphenated names and associates
matching CSS and JavaScript or TypeScript files. Other built-in plugins may
support different source layouts. See [Plugins](/guide/concepts/plugins/) for
FAST component discovery.

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

Install the package into `node_modules/`. Default WebUI discovery derives the
component name from each hyphenated `<component-name>.html` filename, exactly as
for local components. It scans the package's `components/` directory when present,
otherwise the package root. Nested directories are supported; a directory does
not need to repeat the component name.

```text
package.json
components/
  my-button.html
  my-button.css
  my-button.ts
```

This registers `<my-button>`. Matching `.css` provides styles; a matching `.ts`
or `.js` sibling marks only that component as authored. `.spec.ts` files and
package JavaScript exports do not make unrelated components scripted. Import
authored browser registrations through the package's module exports separately.
Scriptless components need neither a registration import nor a projection entry.

When `components/` exists, other package directories such as `dist/` are not
scanned for duplicate source templates. Hidden directories and nested
`node_modules/` are skipped.

**Scoped packages:** A bare scope such as `@reactive-ui` checks each installed
sub-package in the nearest matching scope directory for named HTML components.
An unrelated nearer `node_modules/` does not hide an ancestor's scope.
Packages without component sources are skipped; failures in declared components
are reported rather than silently omitted.
`@scope/*` and `@scope/package/*` can also be used as collection spellings.

**Migration:** Default discovery no longer interprets
`exports["./template-webui.html"]`, `exports["./styles.css"]`, or `customElements`
to identify components. Name templates `<component-name>.html` and styles
`<component-name>.css` instead. Those metadata fields may remain for other
consumers but do not rename or select default WebUI templates.
FAST keeps its separate CEM-based naming and special template/style conventions;
see [Plugins](/guide/concepts/plugins/).

### Local Paths

You can also point to directories outside your app folder:

```bash
webui build ./my-app --out ./dist --components ./shared/components
```

Local path discovery works identically to app directory scanning - HTML files with
hyphenated names are registered as components, matching CSS files are auto-paired,
and a sibling `.ts` or `.js` file marks that component as authored/interactive.

### Caching

npm package discovery results are cached at `~/.webui/cache/components/` and
updated automatically when the selected discovery plugin's source inputs change.
Local path sources are always re-scanned.

See the [CLI Reference](/guide/cli/) for full `--components` usage.
