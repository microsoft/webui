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

### Global options

These flags work with any command:

| Option | Description | Default |
|--------|-------------|---------|
| `--format <FORMAT>` | Output format: `human` (colorized terminal) or `json` (machine-readable diagnostics on stdout) | `human` |

Use `--format json` in editors, CI, or AI/agent tooling that needs to parse build errors programmatically instead of scraping colorized terminal text. See [Error output and exit codes](#error-output-and-exit-codes).

### `webui build`

Build a WebUI application from an app folder.

```bash
webui build [APP] --out <OUT> [--entry <FILE>] [--css <MODE>] [--plugin <NAME>] [--components <SOURCE>]... [--css-file-name-template <TEMPLATE>] [--css-public-base <BASE>] [--legal-comments <MODE>]
```

**Arguments:**

| Argument | Description | Default |
|----------|-------------|---------|
| `APP` | Path to the app folder | `.` (current directory) |
| `--out <OUT>` | Output folder for protocol and assets, or a `.bin` file path to set the protocol filename (e.g. `./dist/app1.bin`) | *(required)* |
| `--entry <FILE>` | Entry HTML file name | `index.html` |
| `--css <STRATEGY>` | CSS delivery strategy: `link`, `style`, or `module` | `link` |
| `--plugin <NAME>` | Load a parser plugin | *(none)* |
| `--dom <STRATEGY>` | DOM strategy: `shadow` or `light` | `shadow` |
| `--components <SOURCE>` | Additional component sources (npm packages or local paths). Repeatable. | *(none)* |
| `--css-file-name-template <TEMPLATE>` | Link-mode CSS filename template. Tokens: `[name]`, `[hash]`, `[ext]` | `[name].[ext]` |
| `--css-public-base <BASE>` | Optional public URL/path prefix for Link-mode CSS hrefs | *(none)* |
| `--legal-comments <MODE>` | Legal comment handling: `inline` preserves legal CSS comments, `none` strips all comments | `inline` |

Path inputs for `APP`, `--state`, and `--servedir` support absolute paths, relative paths, `~/...`, and `file://...` URI-style values.

**CSS Modes:**

| Mode | Behavior |
|------|----------|
| `link` | Emits `<link>` tags referencing external `.css` files. CSS files are copied to the output folder. |
| `style` | Embeds CSS content directly in `<style>` tags inside shadow DOM templates. No separate CSS files are written. |
| `module` | Emits `<script type="importmap">{"imports":{"component":"data:text/css,..."}}</script>` tags that register each component's CSS under a data URI, and adds `shadowrootadoptedstylesheets` to `<template>` tags. The browser shares a single `CSSStyleSheet` across all shadow roots that adopt it. No separate CSS files are written. Based on the [Import Maps](https://html.spec.whatwg.org/multipage/webappapis.html#import-maps) and [CSS Module Scripts](https://github.com/whatwg/html/issues/9572) proposals. **Note:** if a component supplies its own `<template>` wrapper (e.g. to attach `@event` handlers), the wrapper must include `shadowrootadoptedstylesheets="component-name"` — the build fails fast otherwise so adoption can never silently break. |

For long-lived CDN/browser caching in `link` mode, include `[hash]` in the CSS
filename template. `[hash]` is the component CSS file's SHA-256 content hash
truncated to 8 hex characters. The CSS file is still written to `--out`;
`--css-public-base` only changes the href stored in `protocol.bin` and emitted
in `<link>` tags.

**Comment handling:**

WebUI strips HTML comments and CSS comments at build time. Bindings or
directives inside HTML comments are ignored and never produce fragments or
hydration metadata. Inside `<style>` tags, dynamic CSS fragments are valid only
when wrapped as exact CSS block comments, such as `/*{{{tokens.light}}}*/`.
With the default `--legal-comments inline`, CSS comments that contain
`@license` or `@preserve`, or start with `/*!` or `//!`, are preserved inline.
Use `--legal-comments none` to strip all non-signal comments.

**DOM Strategies:**

| Strategy | Behavior |
|----------|----------|
| `shadow` | Components render inside `<template shadowrootmode="open">`. Style encapsulation via Shadow DOM. Default. |
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

# Build link-mode CSS with content-hashed filenames
webui build ./my-app --out ./dist --css-file-name-template "[name]-[hash].[ext]"

# Point generated stylesheet hrefs at a CDN/public asset root
webui build ./my-app --out ./dist \
  --css-file-name-template "[name]-[hash].[ext]" \
  --css-public-base "https://cdn.example.com/assets"

# Build with the WebUI Framework plugin (hydration support)
webui build ./my-app --out ./dist --plugin=webui

# Build with external component packages
webui build ./my-app --out ./dist --components @reactive-ui

# Build with components from a local shared library
webui build ./my-app --out ./dist --components ./shared/components

# Customize the protocol filename (useful when building multiple apps to one folder)
webui build ./src/apps/app1 --out ./dist/app1.bin
webui build ./src/apps/app2 --out ./dist/app2.bin
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
webui serve [APP] --state <FILE> [--servedir <DIR>] [--watch] [--port <PORT>] [--entry <FILE>] [--css <MODE>] [--dom <MODE>] [--plugin <NAME>] [--components <SOURCE>]... [--api-port <PORT>] [--theme <VALUE>] [--css-file-name-template <TEMPLATE>] [--css-public-base <BASE>] [--legal-comments <MODE>]
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
| `--plugin <NAME>` | Load parser + handler plugins (e.g., `webui`) | *(none)* |
| `--dom <STRATEGY>` | DOM strategy: `shadow` or `light` | `shadow` |
| `--components <SOURCE>` | Additional component sources (npm packages or local paths). Repeatable. | *(none)* |
| `--api-port <PORT>` | Proxy route requests to your API server on this port. The dev server forwards navigation requests so your backend can provide real state data. | *(none)* |
| `--theme <VALUE>` | Design token theme: a path to a JSON file or an npm package name. Resolved tokens are injected into the render state. | *(none)* |
| `--css-file-name-template <TEMPLATE>` | Link-mode CSS filename template. Tokens: `[name]`, `[hash]`, `[ext]` | `[name].[ext]` |
| `--css-public-base <BASE>` | Optional public URL/path prefix for Link-mode CSS hrefs | *(none)* |
| `--legal-comments <MODE>` | Legal comment handling: `inline` preserves legal CSS comments, `none` strips all comments | `inline` |

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

# Use the WebUI Framework plugin for hydration
webui serve ./my-app --state ./state.json --plugin=webui --port 3001

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

## Error output and exit codes

When a template has an authoring mistake, the CLI prints a structured diagnostic with a stable error code, the source location, the offending snippet, and an actionable `help:` line:

```
✘ error: invalid <for> each expression [invalid-for-each]
  --> index.html:67:5
    each="person inpeople"
  help: use the form each="item in collection", e.g. each="todo in todos"
```

Where the mistake is likely a typo, the `help:` line suggests the intended name — a misspelled directive attribute (`eahc` → `each`) or an unregistered custom-element tag that closely matches a registered component **in the same namespace** (`<mp-buton>` → `<mp-button>`). A custom element in a different namespace (e.g. a third-party `<md-button>`) is left untouched and passes through to the browser.

### JSON diagnostics

With `--format json`, each error is emitted as a single JSON object on **stdout** (the colorized terminal output is suppressed), so editors, CI, and AI assistants can consume it directly:

```bash
webui build ./my-app --out ./dist --format json
```

```json
{
  "severity": "error",
  "code": "invalid-for-each",
  "message": "invalid <for> each expression",
  "file": "index.html",
  "line": 67,
  "column": 5,
  "snippet": "each=\"person inpeople\"",
  "help": "use the form each=\"item in collection\", e.g. each=\"todo in todos\"",
  "chain": ["Build failed", "Failed to parse index.html", "..."]
}
```

Fields that don't apply to a given error are `null`. The `code` is stable across releases — branch on it rather than on the human-readable `message`.

### Exit codes

The process exit code follows the BSD `sysexits.h` conventions so scripts and CI can branch on the cause:

| Code | Meaning |
|------|---------|
| `0` | Success |
| `1` | Generic failure |
| `2` | Invalid arguments / usage |
| `65` | Template or authoring error (`EX_DATAERR`) |
| `66` | Missing input: app folder, `--state` file, `--servedir`, or entry file (`EX_NOINPUT`) |
| `69` | Requested `--port` is already in use (`EX_UNAVAILABLE`) |
| `74` | I/O error reading or writing files (`EX_IOERR`) |

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

The `--plugin` flag loads framework-specific extensions that customize both parsing and rendering behavior. The available plugin identifiers are listed in the [Plugins](/guide/concepts/plugins/) reference. No plugin is enabled by default — output is plain SSR HTML unless one is selected.

```bash
# Load a plugin by name
webui build ./my-app --out ./dist --plugin=<name>
webui serve ./my-app --state ./state.json --plugin=<name>
```

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
