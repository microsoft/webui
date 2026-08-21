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
| `--api-port` | *(none)* | Proxy API requests; JSON provides buffered state and `application/x-webui-stream` drives progressive boundaries |
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
- API proxy when `--api-port` is set. Backends may return JSON state or a
  versioned, newline-delimited `application/x-webui-stream` control response;
  the CLI retains the Rust renderer, caps precommit output staging at 4,000,000
  bytes, and cancels the backend when the browser disconnects.

The control response uses version 2 and exactly three command types:

```json
{"type":"start","version":2,"state":{"title":"Initial state"}}
{"type":"resume","boundary":{"owner":"index.html","name":"hero"},"mode":"updatable","state":{}}
{"type":"update","boundary":{"owner":"index.html","name":"hero"},"state":{"status":"ready"}}
```

`start` is first and supplies the initial object state. Each `resume` must echo
the pending runtime descriptor's `owner`, `name`, and optional string-or-number
`key`; `declarationId` may also be supplied for validation. The CLI retains
committed descriptors, so `update` uses the same target without a reverse
acknowledgement channel. Ambiguous unkeyed update targets are rejected.

The CLI drives the Rust step machine as follows:

| Rust step state | CLI action |
|---|---|
| descriptor present | Wait for the matching backend `resume`, then call `resume` |
| no descriptor and not done | Call `advance` internally |
| done | Complete the browser response |

Rust `resume` emits only the pending boundary through its checkpoint. The
following internal `advance` emits parent or tail bytes through the next
descriptor or terminal. The backend does not send an `advance` control record.
It closes its NDJSON body after sending the resume for the final descriptor;
the CLI's final `advance` completes the response. There is no terminal control
command.

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
