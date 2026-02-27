# Examples

This directory contains runnable WebUI examples.

## Structure

- `app/` — source app examples (templates, assets, data)
- `integration/` — host-language integrations that load `protocol.bin` and render HTML

Current entries:

- `app/hello-world`
- `integration/node`
- `integration/rust`

## Quick Start

From the workspace root, build the hello-world app output:

```bash
cargo run -p webui-cli -- build examples/app/hello-world/templates --out examples/app/hello-world/dist
```

Then run an integration example:

### Node

```bash
cd examples/integration/node
node index.js
```

### Rust

```bash
cd examples/integration/rust
cargo run -- ../../app/hello-world/dist/protocol.bin ../../app/hello-world/data/state.json
```

## More Details

See integration-specific READMEs:

- [integration/node/README.md](integration/node/README.md)
- [integration/rust/README.md](integration/rust/README.md)
