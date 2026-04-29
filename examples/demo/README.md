# WebUI Demo Shell

A unified demo that hosts all WebUI example apps behind a single Rust reverse-proxy.
The shell UI itself is a real WebUI app — the `<for>` loop, `?disabled` boolean
attributes on the prev/next buttons, and `?selected` on the dropdown options are
all driven by server-rendered state.

## Architecture

```
Browser → :8080 → Demo Shell (Rust/actix-web)
                    ├── /              → SSR shell (examples/demo/src) via WebUIHandler
                    ├── /_shell/*      → Bundled shell client (examples/demo/dist/index.js)
                    ├── /{app}/*       → Reverse proxy → internal app server
                    ├── /api/apps      → App registry (JSON)
                    └── /health        → Health status
```

The shell's protocol is compiled in-process at startup via `webui::build()`.
Each request renders that cached protocol against a fresh JSON state derived
from the discovered app registry — so the dropdown, prev/next disabled flags,
badge, description, counter, and source link are populated entirely by server
state. A small client script (`src/index.ts`) handles user-driven navigation
by mutating the existing DOM.

Each example app runs on a dynamically assigned internal port. The shell
auto-discovers them by scanning for `demo.toml` files in the apps directory.

## Quick Start (Docker)

```bash
# From the repository root:
docker build -t webui-demo -f examples/demo/Dockerfile .
docker run -p 8080:8080 webui-demo
```

Then open http://localhost:8080.

## Layout

```
examples/demo/
├── src/
│   ├── index.html          ← Shell entry template (uses <for>, ?disabled, etc.)
│   └── index.ts            ← Client navigation script (vanilla JS)
├── data/
│   └── state.json          ← Sample state for solo dev (`webui serve`)
├── dist/
│   └── index.js            ← Bundled client (produced by `pnpm build`)
├── server/                 ← Rust reverse-proxy + SSR host (binary: demo-shell)
└── README.md
```

## `demo.toml` Schema

Each example app has a `demo.toml` file that describes how to run it:

```toml
name = "Calculator"
description = "A scientific calculator using the fast-v3 rendering plugin"
backend = "rust"                    # rust | node | rust-and-node | wasm | dotnet

[server]
type = "webui-cli"                  # webui-cli | custom-binary
plugin = "fast-v3"                  # fast-v3 | webui | fast-v2 | fast (fast/fast-v2 are deprecated FAST 2)
source = "src"                      # Source directory
servedir = "dist"                   # Static assets directory
state = "data/state.json"           # Optional state file
theme = "@microsoft/webui-examples-theme"  # Optional theme

# Optional: separate API server
[api]
type = "node"                       # node (more types in future)
entry = "dist/api.js"               # Built entry point
port-offset = 10                    # API port = app port + offset
```

### Adding a New Example App

1. Create your app in `examples/app/{my-app}/`
2. Add a `demo.toml` file with the metadata above
3. Rebuild the Docker image — the shell auto-discovers it

## CLI Options

```
demo-shell [OPTIONS]

Options:
  --port <PORT>            Port to listen on [default: 8080]
  --apps-dir <APPS_DIR>    Directory with app subdirectories [default: ./apps]
  --base-port <BASE_PORT>  Base port for dynamic assignment [default: 3100]
  --shell-dir <SHELL_DIR>  Shell WebUI app directory [default: ./examples/demo]
```

## Local Development (without Docker)

Build the shell client bundle once, then start the shell:

```bash
# 1. Build the shell client bundle
cd examples/demo && pnpm install && pnpm build && cd ../..

# 2. Make sure the webui CLI is on your PATH (or install it):
cargo install --path crates/webui-cli
#   …or run with:  PATH="$PWD/target/debug:$PATH" cargo run -p demo-shell -- ...

# 3. Pre-build each example app's client assets (run `pnpm build`
#    in each examples/app/<slug>/).

# 4. Run the shell
cargo run -p demo-shell -- --port 8080 --apps-dir examples/app --shell-dir examples/demo
```

To iterate on the shell template only (no proxy / app discovery), use:

```bash
cd examples/demo && pnpm start:server    # serves examples/demo/src on :3099
```

## Discovered Apps

| App | Backend | Description |
|-----|---------|-------------|
| Calculator | Rust | Scientific calculator (fast plugin) |
| Commerce | Rust | Full e-commerce marketplace (custom Actix server) |
| Contact Book | Rust + Node | CRUD contact manager with REST API |
| Hello World | Rust | Minimal WebUI starter app |
| Routes | Rust + Node | Multi-page routed app with Node.js API |
| Todo (Fast) | Rust | Todo app (fast plugin) |
| Todo (WebUI) | Rust | Todo app (webui plugin) |
