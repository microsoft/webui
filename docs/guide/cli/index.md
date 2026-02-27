# CLI Reference

The `webui` command-line tool is the primary way to build WebUI applications. It takes your app folder containing HTML templates and web components, and produces the WebUI protocol output ready for server-side rendering.

## Installation

Build from source:

```bash
git clone https://github.com/microsoft/webui.git
cd webui
cargo build --release
```

The `webui` binary will be available at `target/release/webui`.

## Commands

### `webui build`

Build a WebUI application from an app folder.

```bash
webui build [APP] --out <OUT> [--entry <FILE>] [--css <MODE>]
```

**Arguments:**

| Argument | Description | Default |
|----------|-------------|---------|
| `APP` | Path to the app folder | `.` (current directory) |
| `--out <OUT>` | Output folder for protocol and assets | *(required)* |
| `--entry <FILE>` | Entry HTML file name | `index.html` |
| `--css <MODE>` | CSS delivery strategy: `external` or `inline` | `external` |

Path inputs for `APP`, `--state`, and `--servedir` support absolute paths, relative paths, `~/...`, and `file://...` URI-style values.

**CSS Modes:**

| Mode | Behavior |
|------|----------|
| `external` | Emits `<link>` tags referencing external `.css` files. CSS files are copied to the output folder. |
| `inline` | Embeds CSS content directly in `<style>` tags inside shadow DOM templates. No separate CSS files are written. |

**Examples:**

```bash
# Build from current directory
webui build --out ./dist

# Build a specific app folder
webui build ./my-app --out ./dist

# Use a custom entry file
webui build ./my-app --out ./dist --entry home.html

# Build with inline CSS (no external CSS files)
webui build ./my-app --out ./dist --css inline
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

### `webui start`

Start a development server that builds, renders, and serves a WebUI application. Enable live reload with `--watch`.

```bash
webui-cli start [APP] --state <FILE> [--servedir <DIR>] [--watch] [--port <PORT>] [--entry <FILE>] [--css <MODE>]
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
| `--css <MODE>` | CSS delivery strategy: `external` or `inline` | `external` |

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
webui-cli start . --state ./state.json --servedir ./assets

# Start serving a specific templates directory
webui-cli start ./examples/app/hello-world/templates --state ./examples/app/hello-world/data/state.json --servedir ./examples/app/hello-world/assets --watch

# Use a custom port
webui-cli start ./my-app --state ./state.json --servedir ./assets --port 9090 --watch

# Use inline CSS mode
webui-cli start ./my-app --state ./state.json --servedir ./assets --css inline --watch
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
- Discovery is recursive — components in subdirectories are also found

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
├── my-card.css         # Component CSS (--css external only)
└── nav-bar.css         # Component CSS (--css external only)
```

With `--css inline`, only `protocol.bin` is written — CSS is embedded directly in the protocol's template fragments.

### protocol.bin

The protocol file contains a serialized `WebUIProtocol` structure (protobuf binary) with all parsed fragments. This file is consumed by a [platform handler](/guide/concepts/handlers/) at runtime to render HTML with your application state.

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

## Next Steps

- [Hello World Tutorial](/tutorials/hello-world) — Build your first WebUI app
- [Components](/guide/concepts/components/) — Learn about web components
- [Template Directives](/guide/concepts/directives/) — `<for>`, `<if>`, and `{{}}`
- [Platform Handlers](/guide/concepts/handlers/) — Render protocols with state at runtime
