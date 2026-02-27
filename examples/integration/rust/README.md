# WebUI Rust Example

Minimal example showing how to use WebUI as a Rust library to render a pre-built protocol with state data.

## Prerequisites

Build the hello-world app first:

```bash
cargo run -p webui-cli -- build ../../app/hello-world/templates --out ../../app/hello-world/dist
```

## Usage

```bash
cargo run -- ../../app/hello-world/dist/protocol.bin ../../app/hello-world/data/state.json
```

This loads `protocol.bin`, passes the state from `state.json`, and prints the rendered HTML to stdout.
