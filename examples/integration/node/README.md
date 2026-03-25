# WebUI Node.js Integration Example

Minimal example showing how to use the `@microsoft/webui` npm package to build
templates and render HTML with state data — all from Node.js.

## Prerequisites

1. Build the native addon:

```bash
cargo build -p microsoft-webui-node
```

2. Build the `@microsoft/webui` package:

```bash
pnpm --filter @microsoft/webui build
```

3. Install workspace dependencies:

```bash
pnpm install
```

## Usage

Build the hello-world app and render it with state data:

```bash
node index.js
```

Or render a pre-built protocol with custom state:

```bash
node index.js ../../app/hello-world/dist/protocol.bin ../../app/hello-world/data/state.json
```

This uses the `@microsoft/webui` package API (`build()`, `render()`, `renderStream()`) which
automatically resolves the native addon from the workspace build output.
