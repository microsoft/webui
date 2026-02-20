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
webui build [APP] --out <OUT> [--entry <FILE>]
```

**Arguments:**

| Argument | Description | Default |
|----------|-------------|---------|
| `APP` | Path to the app folder | `.` (current directory) |
| `--out <OUT>` | Output folder for protocol and assets | *(required)* |
| `--entry <FILE>` | Entry HTML file name | `index.html` |

**Examples:**

```bash
# Build from current directory
webui build --out ./dist

# Build a specific app folder
webui build ./my-app --out ./dist

# Use a custom entry file
webui build ./my-app --out ./dist --entry home.html
```

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
├── protocol.json       # The WebUI protocol (JSON)
├── my-card.css         # Component CSS (copied)
└── nav-bar.css         # Component CSS (copied)
```

### protocol.json

The protocol file contains a serialized `WebUIProtocol` structure with all parsed fragments. This file is consumed by a [platform handler](/guide/concepts/handlers/) at runtime to render HTML with your application state.

```json
{
  "fragments": {
    "index.html": [
      { "type": "raw", "value": "<h1>Hello, " },
      { "type": "signal", "value": "name", "raw": false },
      { "type": "raw", "value": "!</h1>" },
      { "type": "for", "item": "item", "collection": "items", "fragmentId": "for-1" }
    ],
    "for-1": [
      { "type": "component", "fragmentId": "my-card" },
      { "type": "signal", "value": "item.title", "raw": false }
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
