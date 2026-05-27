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
webui build [APP] --out <DIR> [--entry <FILE>] [--css <MODE>] [--plugin <NAME>] [--css-file-name-template <TEMPLATE>] [--css-public-base <BASE>]
```

| Option | Default | Description |
|--------|---------|-------------|
| `APP` | `.` | Template/component directory |
| `--out` | *(required)* | Output directory for protocol.bin + CSS |
| `--entry` | `index.html` | Entry HTML file |
| `--css` | `link` | CSS mode: `link` (external files) or `style` (inline) |
| `--plugin` | *(none)* | Plugin identifier (see [Plugins](https://microsoft.github.io/webui/guide/concepts/plugins/) for available identifiers) |
| `--css-file-name-template` | `[name].[ext]` | Link-mode CSS filename template. Tokens: `[name]`, `[hash]`, `[ext]` |
| `--css-public-base` | *(none)* | Optional base URL/path prepended to Link-mode stylesheet hrefs |

```bash
webui build ./src --out ./dist
webui build ./src --out ./dist --plugin webui --css style
webui build ./src --out ./dist --css-file-name-template "[name]-[hash].[ext]"
webui build ./src --out ./dist --css-file-name-template "[name]-[hash].[ext]" --css-public-base "https://cdn.example.com/assets"
```

### `webui serve`

Start a development server with live rebuild and HMR.

```bash
webui serve [APP] [--state <FILE>] [--servedir <DIR>] [--port <PORT>] [--api-port <PORT>] [--plugin <NAME>] [--watch] [--css-file-name-template <TEMPLATE>] [--css-public-base <BASE>]
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
| `--css-file-name-template` | `[name].[ext]` | Link-mode CSS filename template. Tokens: `[name]`, `[hash]`, `[ext]` |
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
