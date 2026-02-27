# webui-cli

`webui-cli` provides the `webui` binary used to build, inspect, and locally serve WebUI apps.

## Build and run

From the workspace root:

```bash
cargo run -p webui-cli -- <command> [options]
```

Or build a release binary:

```bash
cargo build -p webui-cli --release
./target/release/webui <command> [options]
```

## Commands

### `webui build`

Builds protocol output from templates/components.

```bash
webui build [APP] --out <OUT> [--entry <FILE>] [--css <MODE>]
```

- `APP`: template/component directory (default `.`)
- `--out`: output directory (required)
- `--entry`: entry HTML file (default `index.html`)
- `--css`: `external` or `inline` (default `external`)

Examples:

```bash
webui build ./examples/app/hello-world/templates --out ./dist
webui build ./examples/app/hello-world/templates --out ./dist --entry index.html --css inline
```

### `webui inspect`

Converts a `protocol.bin` file to JSON and prints it to stdout.

```bash
webui inspect <FILE>
```

Example:

```bash
webui inspect ./dist/protocol.bin
```

### `webui start`

Starts a dev server with build+render. Live reload/HMR is optional via `--watch`.

```bash
webui-cli start [APP] --state <FILE> [--servedir <DIR>] [--watch] [--port <PORT>] [--entry <FILE>] [--css <MODE>]
```

- `APP`: template/component directory (default `.`)
- `--state`: JSON state file (required)
- `--servedir`: optional static assets directory served at `/*`
- `--watch`: enable file watching + HMR (disabled by default)
- `--port`: server port (default `3000`)
- `--entry`: entry HTML file (default `index.html`)
- `--css`: `external` or `inline` (default `external`)

Behavior:

- Serves rendered HTML at `/` and `/index.html`
- Serves static files from `--servedir` at `/*` when provided
- Exposes HMR polling endpoint at `/hmr` when `--watch` is enabled
- Watches app/state/assets and rebuilds when changes are detected when `--watch` is enabled

Example:

```bash
webui-cli start ./examples/app/hello-world/templates \
  --state ./examples/app/hello-world/data/state.json \
  --servedir ./examples/app/hello-world/assets \
  --watch \
  --port 3000
```

## Path handling

`APP` and `--state` support relative paths, absolute paths, and `~/...` expansion. `--servedir` supports the same formats when provided.

## Typical app layout

```text
my-app/
├── index.html
├── my-card.html
├── my-card.css
└── state.json
```

## Output

`webui build` writes:

- `protocol.bin`
- component CSS files (only when `--css external`)

Use `webui inspect` to view a JSON representation for debugging.
