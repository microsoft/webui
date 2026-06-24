# microsoft-webui-cli

Command-line tool for the [WebUI](https://github.com/microsoft/webui) framework — build, serve, and inspect WebUI applications.

## Install

```bash
cargo install microsoft-webui-cli
```

This installs the `webui` binary.

## Commands

### `webui build`

Build a WebUI application into a compiled protocol and CSS files.

```bash
webui build [APP] --out <DIR> [--entry <FILE>] [--css <MODE>] [--plugin <NAME>] [--asset-file-name-template <TEMPLATE>] [--css-public-base <BASE>]
```

| Option | Default | Description |
|--------|---------|-------------|
| `APP` | `.` | Template/component directory |
| `--out` | *(required)* | Output directory for protocol.bin + CSS, or a `.bin` file path to customize the protocol filename (e.g. `./dist/app1.bin`) |
| `--entry` | `index.html` | Entry HTML file |
| `--css` | `link` | CSS mode: `link` (external files) or `style` (inline) |
| `--plugin` | *(none)* | Plugin identifier (see [Plugins](https://microsoft.github.io/webui/guide/concepts/plugins/) for available identifiers) |
| `--asset-file-name-template` | `[name].[ext]` | Emitted asset filename template. Tokens: `[name]`, `[hash]`, `[ext]` |
| `--css-public-base` | *(none)* | Optional base URL/path prepended to Link-mode stylesheet hrefs |

```bash
webui build ./src --out ./dist
webui build ./src --out ./dist --plugin webui --css style
webui build ./src --out ./dist/app1.bin
webui build ./src --out ./dist --asset-file-name-template "[name]-[hash].[ext]"
webui build ./src --out ./dist --asset-file-name-template "[name]-[hash].[ext]" --css-public-base "https://cdn.example.com/assets"
```

### `webui serve`

Start a development server with live rebuild and HMR.

```bash
webui serve [APP] [--state <FILE>] [--servedir <DIR>] [--port <PORT>] [--api-port <PORT>] [--plugin <NAME>] [--watch] [--asset-file-name-template <TEMPLATE>] [--css-public-base <BASE>]
```

| Option | Default | Description |
|--------|---------|-------------|
| `APP` | `.` | Template/component directory |
| `--state` | *(none)* | JSON state file for rendering |
| `--servedir` | *(none)* | Static assets directory served at `/*` |
| `--port` | `3000` | Server port |
| `--api-port` | *(none)* | Proxy API requests to this port |
| `--plugin` | *(none)* | Plugin identifier (see [Plugins](https://microsoft.github.io/webui/guide/concepts/plugins/) for available identifiers) |
| `--watch` | off | Enable file watching + HMR |
| `--asset-file-name-template` | `[name].[ext]` | Emitted asset filename template. Tokens: `[name]`, `[hash]`, `[ext]` |
| `--css-public-base` | *(none)* | Optional base URL/path prepended to Link-mode stylesheet hrefs |

```bash
webui serve ./src --state ./data/state.json --port 3000 --watch
webui serve ./src --plugin webui --servedir ./dist --port 3004 --api-port 3014 --watch
```

Features:
- Renders HTML at `/` and all route paths
- Serves static files from `--servedir`
- JSON partials for client-side navigation (`Accept: application/json`)
- HMR polling at `/hmr` when `--watch` is enabled
- API proxy when `--api-port` is set

### `webui inspect`

Convert a compiled protocol to JSON for debugging.

```bash
webui inspect <FILE>
```

```bash
webui inspect ./dist/protocol.bin
```

### `webui desktop`

Run desktop commands through the desktop sidecar backend. `webui` remains the
only user-facing CLI; the sidecar is resolved automatically from the installed
desktop support package, next to the `webui` binary, or from the workspace during
local development. Set `WEBUI_DESKTOP_BINARY` only to override sidecar discovery.

```bash
webui desktop build ./src \
  --state ./data/state.json \
  --servedir ./dist \
  --out ./desktop-bundle \
  --plugin webui \
  --devtools
```

The sidecar currently creates immutable desktop bundles with `protocol.bin`,
copied assets, startup state, `manifest.webui-desktop.json`, and SHA-256
integrity hashes. Native window backends and platform package emitters are
implemented in the desktop sidecar so the default CLI stays lean.

```bash
webui desktop package ./my-app --target macos-app --out ./packages
webui desktop package ./desktop-bundle --target macos-app --out ./packages \
  --runner ./target/release/my-desktop-host
```

The Rust packager currently writes runnable macOS `.app` bundles and portable
folder layouts. For app roots, the sidecar reads `webuiDesktop` from
`package.json`, runs configured web build scripts, builds the app-specific Cargo
runner crate, stages non-generated assets, builds the bundle, and packages the
runner-backed app. Use `--runner` for lower-level existing-bundle flows with
route providers or typed IPC commands; omitting it packages the generic sidecar
for file-backed/static seed-state bundles. Installer targets return actionable
missing-tool diagnostics until the platform packagers are enabled.

Use `--devtools` on desktop build/run to make development webviews inspectable.
On macOS, inspect from Safari's Develop menu.

Rust desktop apps that need dynamic route data should use
`webui_desktop::DesktopApp::builder(...).route(...)` in their host binary. The
CLI `--state` flag is a file-backed fallback for simple demos.

## App Layout

```
my-app/
├── src/
│   ├── index.html          # entry template
│   ├── my-card.html         # component template
│   └── my-card.css          # component styles
├── data/
│   └── state.json           # render state
└── dist/                    # build output
    ├── protocol.bin
    └── my-card.css
```

## License

MIT
