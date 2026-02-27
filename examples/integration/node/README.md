# WebUI Node.js Example

Minimal example showing how to use the `webui-node` native addon to render a pre-built protocol with state data.

## Prerequisites

1. Build the native addon:

```bash
cargo build -p webui-node
```

2. Build the hello-world app:

```bash
cargo run -p webui-cli -- build ../../app/hello-world/templates --out ../../app/hello-world/dist
```

## Usage

```bash
node index.js
```

Or with custom paths:

```bash
node index.js ../../app/hello-world/dist/protocol.bin ../../app/hello-world/data/state.json
```

This loads `protocol.bin`, passes the state from `state.json`, and prints the rendered HTML to stdout.
