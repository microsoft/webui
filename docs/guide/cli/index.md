# CLI Reference

The `webui` command-line tool is the primary way to build WebUI applications. It takes your app folder containing HTML templates and web components, and produces the WebUI protocol output ready for server-side rendering.

## Installation

Install via npm:

```bash
npm install @microsoft/webui
```

Or install via Cargo for standalone CLI use:

```bash
cargo install microsoft-webui-cli
```

## Commands

### `webui build`

Build a WebUI application from an app folder.

```bash
webui build [APP] --out <OUT> [--entry <FILE>] [--css <MODE>] [--plugin <NAME>] [--components <SOURCE>]...
```

**Arguments:**

| Argument | Description | Default |
|----------|-------------|---------|
| `APP` | Path to the app folder | `.` (current directory) |
| `--out <OUT>` | Output folder for protocol and assets | *(required)* |
| `--entry <FILE>` | Entry HTML file name | `index.html` |
| `--css <STRATEGY>` | CSS delivery strategy: `link`, `style`, or `module` | `link` |
| `--plugin <NAME>` | Load a parser plugin (e.g., `fast-v3`) | *(none)* |
| `--dom <STRATEGY>` | DOM strategy: `shadow` or `light` | `shadow` |
| `--components <SOURCE>` | Additional component sources (npm packages or local paths). Repeatable. | *(none)* |

Path inputs for `APP`, `--state`, and `--servedir` support absolute paths, relative paths, `~/...`, and `file://...` URI-style values.

**CSS Modes:**

| Mode | Behavior |
|------|----------|
| `link` | Emits `<link>` tags referencing external `.css` files. CSS files are copied to the output folder. |
| `style` | Embeds CSS content directly in `<style>` tags inside shadow DOM templates. No separate CSS files are written. |
| `module` | Emits `<style type="module" specifier="component">` definitions and adds `shadowrootadoptedstylesheets` to `<template>` tags. The browser shares a single `CSSStyleSheet` across all shadow roots that adopt it. No separate CSS files are written. Based on the [Declarative CSS Module Scripts](https://github.com/MicrosoftEdge/MSEdgeExplainers/blob/main/ShadowDOM/explainer.md) proposal. |

**DOM Strategies:**

| Strategy | Behavior |
|----------|----------|
| `shadow` | Components render inside `<template shadowrootmode="open" shadowroot="open">`. Style encapsulation via Shadow DOM. Default. The mode and any other `shadowroot*` attribute on a user-supplied wrapping `<template>` are preserved (e.g. `shadowrootmode="closed"`, `shadowrootclonable`, `shadowrootdelegatesfocus`). The legacy `shadowroot` attribute is always emitted alongside `shadowrootmode` for older user agents. Under the FAST plugins (`fast-v2`/`fast-v3`) the `shadowroot*` attributes are placed on the outer `<f-template>` element instead of the inner `<template>`. |
| `light` | Components render as direct children. No shadow boundary. 26% faster FCP on high-component-count pages. |

See [Performance - Light DOM vs Shadow DOM](/guide/concepts/performance#light-dom-vs-shadow-dom) for benchmarks and guidance.

**Examples:**

```bash
# Build from current directory
webui build --out ./dist

# Build a specific app folder
webui build ./my-app --out ./dist

# Use a custom entry file
webui build ./my-app --out ./dist --entry home.html

# Build with style CSS (no external CSS files)
webui build ./my-app --out ./dist --css style

# Build with the @microsoft/fast-element 3.x plugin (hydration support)
webui build ./my-app --out ./dist --plugin=fast-v3

# Build with external component packages
webui build ./my-app --out ./dist --components @reactive-ui

# Build with components from a local shared library
webui build ./my-app --out ./dist --components ./shared/components
```

### `webui inspect`

Inspect a `protocol.bin` file by converting it to JSON and printing to stdout. Useful for debugging and piping to tools like `jq`.

```bash
webui inspect <FILE>
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `FILE` | Path to a `protocol.bin` file |

**Examples:**

```bash
# Inspect a protocol file
webui inspect dist/protocol.bin

# Pretty-print a specific fragment with jq
webui inspect dist/protocol.bin | jq '.fragments["index.html"]'

# Count total fragments
webui inspect dist/protocol.bin | jq '.fragments | keys | length'
```

### `webui serve`

Start a development server that builds, renders, and serves a WebUI application. Enable live reload with `--watch`.

```bash
webui serve [APP] --state <FILE> [--servedir <DIR>] [--watch] [--port <PORT>] [--entry <FILE>] [--css <MODE>] [--dom <MODE>] [--plugin <NAME>] [--components <SOURCE>]... [--api-port <PORT>] [--theme <VALUE>]
```

**Arguments:**

| Argument | Description | Default |
|----------|-------------|---------|
| `APP` | Path to the template/component directory | `.` (current directory) |
| `--state <FILE>` | Path to JSON state file for rendering | *(required)* |
| `--servedir <DIR>` | Directory served at `/*` | *(optional)* |
| `--watch` | Enable file watching + HMR | `false` |
| `--port <PORT>` | Port to bind the development server | `3000` |
| `--entry <FILE>` | Entry HTML file name | `index.html` |
| `--css <MODE>` | CSS delivery strategy: `link`, `style`, or `module` | `link` |
| `--plugin <NAME>` | Load parser + handler plugins (e.g., `fast-v3`) | *(none)* |
| `--dom <STRATEGY>` | DOM strategy: `shadow` or `light` | `shadow` |
| `--components <SOURCE>` | Additional component sources (npm packages or local paths). Repeatable. | *(none)* |
| `--api-port <PORT>` | Proxy route requests to your API server on this port. The dev server forwards navigation requests so your backend can provide real state data. | *(none)* |
| `--theme <VALUE>` | Design token theme: a path to a JSON file or an npm package name. Resolved tokens are injected into the render state. | *(none)* |

The `APP` directory should contain your entry HTML and component files.

**What it does:**

1. Builds the protocol from your `APP` directory (no separate `webui build` step needed)
2. Renders the entry template with state data
3. Serves the rendered HTML with an injected live-reload script
4. If `--watch` is enabled, watches app, state, and asset files for changes
5. If `--watch` is enabled, automatically rebuilds and re-renders when files change
6. If `--watch` is enabled, connected browsers reload automatically via the polling HMR backend

**Examples:**

```bash
# Start serving the current directory
webui serve . --state ./state.json --servedir ./assets

# Start serving a specific templates directory
webui serve ./examples/app/hello-world/templates --state ./examples/app/hello-world/data/state.json --servedir ./examples/app/hello-world/assets --watch

# Use a custom port
webui serve ./my-app --state ./state.json --servedir ./assets --port 9090 --watch

# Use style CSS mode
webui serve ./my-app --state ./state.json --servedir ./assets --css style --watch

# Use the @microsoft/fast-element 3.x plugin for hydration
webui serve ./my-app --state ./state.json --plugin=fast-v3 --port 3001

# Dev server with external components (--watch watches local paths)
webui serve ./my-app --state ./state.json --components @reactive-ui --watch

# Proxy route requests to your API server (e.g. Express on port 4000)
webui serve ./my-app --state ./state.json --api-port 4000 --watch

# Apply a design token theme from an npm package
webui serve ./my-app --state ./state.json --theme @my-org/brand-tokens --watch

# Apply a design token theme from a local JSON file
webui serve ./my-app --state ./state.json --theme ./themes/dark.json --watch
```

**Routes:**

| Path | Description |
|------|-------------|
| `/` or `/index.html` | Rendered HTML with live-reload script |
| `/*` | Static files from `--servedir` (when provided) |
| `/hmr` | HMR version endpoint (polling backend, only when `--watch`) |

## App Folder Structure

The CLI expects your app folder to contain an entry HTML file and optionally web component files:

```
my-app/
├── index.html          # Entry template (or specify with --entry)
├── my-card.html        # Web component: <my-card>
├── my-card.css         # Component styles (auto-discovered)
├── nav-bar.html        # Web component: <nav-bar>
├── nav-bar.css         # Component styles
├── styles.css          # Global styles
└── app.js              # Client-side scripts
```

### Component Discovery

The CLI automatically discovers web components in your app folder:

- **HTML files with a hyphen** in the name are treated as components (e.g., `my-card.html` → `<my-card>`)
- **CSS files** with the same name are automatically paired (e.g., `my-card.css`)
- Components are registered and available for use in your templates
- Discovery is recursive - components in subdirectories are also found

### Entry Template

Your entry HTML file is a standard HTML document using WebUI directives:

```html
<!DOCTYPE html>
<html lang="en">
<head>
    <title>My App</title>
    <link rel="stylesheet" href="styles.css">
</head>
<body>
    <h1>Hello, {{name}}!</h1>

    <for each="item in items">
        <my-card>{{item.title}}</my-card>
    </for>

    <if condition="showFooter">
        <footer>Thanks for visiting</footer>
    </if>
</body>
</html>
```

## Build Output

The `--out` folder will contain:

```
dist/
├── protocol.bin        # The WebUI protocol (protobuf binary)
├── my-card.css         # Component CSS (--css link only)
└── nav-bar.css         # Component CSS (--css link only)
```

With `--css style`, only `protocol.bin` is written - CSS is embedded directly in the protocol's template fragments.

### protocol.bin

The protocol file contains a serialized `WebUIProtocol` structure (protobuf binary) with all parsed fragments. This file is consumed by a [platform handler](/guide/integrations/) at runtime to render HTML with your application state.

The binary format is not human-readable. The equivalent proto schema structure looks like:

```protobuf
// WebUIProtocol
fragments {
  key: "index.html"
  value: FragmentList {
    fragments: [
      Raw { value: "<h1>Hello, " },
      Signal { value: "name", raw: false },
      Raw { value: "!</h1>" },
      For { item: "item", collection: "items", fragment_id: "for-1" }
    ]
  }
  key: "for-1"
  value: FragmentList {
    fragments: [
      Component { fragment_id: "my-card" },
      Signal { value: "item.title", raw: false }
    ]
  }
}
```

## Error Messages

The CLI provides helpful error messages with suggestions:

```
  ✘ Failed to read /path/to/app/index.html
  caused by: No such file or directory (os error 2)

  hint: Try using --entry <file> to specify a different entry file
```

```
  ✘ App folder not found: /nonexistent/path
  caused by: No such file or directory (os error 2)

  hint: Check that the app folder path exists
```

## Plugins

The `--plugin` flag loads framework-specific extensions that customize both parsing and rendering behavior.

### Available Plugins

| Plugin | Description |
|--------|-------------|
| `webui` | WebUI Framework compiled templates and hydration markers. |
| `fast-v3` | @microsoft/fast-element 3.x hydration support for new FAST apps. Parser skips runtime attrs, emits binding data, and injects `<f-template>` wrappers. Handler injects `<!--fe:b-->`, `<!--fe:/b-->`, `<!--fe:r-->`, `<!--fe:/r-->`, and `data-fe="COUNT"` markers. |
| `fast-v2` | Deprecated @microsoft/fast-element 2.x compatibility. Emits legacy `<!--fe-b$$...-->`, `<!--fe-repeat$$...-->`, `data-fe-b-INDEX`, and `data-fe-c-INDEX-COUNT` markers. |
| `fast` | Deprecated compatibility alias for `fast-v2`. Use `fast-v3` for @microsoft/fast-element 3.x migrations. |

No plugin is enabled by default. Select `fast-v3` explicitly for @microsoft/fast-element 3.x apps; `fast` remains accepted only to avoid silently changing existing @microsoft/fast-element 2.x output.

See [Plugins](/guide/concepts/plugins/) for detailed documentation.

## External Component Sources

The `--components` flag lets you discover components from npm packages or local directories outside your app folder. This is useful for shared component libraries.

### npm Packages

Pass an npm package name. The package must already be installed in `node_modules/`.

```bash
# Single package
webui build ./my-app --out ./dist --components my-widget

# Scoped package (discovers all sub-packages)
webui build ./my-app --out ./dist --components @reactive-ui

# Specific scoped sub-package
webui build ./my-app --out ./dist --components @reactive-ui/button
```

**npm package requirements:**

The package's `package.json` must have:

| Field | Purpose |
|-------|---------|
| `exports["./template-webui.html"]` | Path to the component's HTML template |
| `exports["./styles.css"]` | Path to the component's CSS (optional) |
| `customElements` | Path to a [Custom Elements Manifest](https://github.com/webcomponents/custom-elements-manifest) JSON file |

The Custom Elements Manifest provides the component tag name via `modules[].declarations[].tagName`.

**Resolution:** The CLI searches for `node_modules/` by walking up from the app directory, matching Node.js module resolution behavior. Symlinks (pnpm, npm workspaces) are resolved automatically.

### Local Paths

Pass a filesystem path to discover components the same way the app directory is scanned.

```bash
# Relative path
webui build ./my-app --out ./dist --components ./shared/components

# Absolute path
webui build ./my-app --out ./dist --components /libs/ui-kit
```

### Multiple Sources

Combine multiple `--components` flags:

```bash
webui build ./my-app --out ./dist \
  --components @reactive-ui \
  --components ./shared/components \
  --components my-widget
```

### Caching

Discovered npm package components are cached at `~/.webui/cache/components/` to avoid re-traversing on every build. The cache is automatically invalidated when a package's `package.json` changes. Local path sources are always re-scanned.

## Next Steps

- [Hello World Tutorial](/tutorials/hello-world) - Build your first WebUI app
- [Components](/guide/concepts/components/) - Learn about web components
- [Template Directives](/guide/concepts/directives/) - `<for>`, `<if>`, and `{{}}`
- [Platform Handlers](/guide/integrations/) - Render protocols with state at runtime
