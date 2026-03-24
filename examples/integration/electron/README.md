# WebUI Electron Integration

Wraps any pre-built WebUI app in a frameless Electron desktop window using the `webui-node` native addon.

## Prerequisites

1. Build the native addon:

```bash
cargo build -p microsoft-webui-node --release
```

2. Build a WebUI app (e.g. hello-world):

```bash
cargo run -p microsoft-webui-cli -- build ../../app/hello-world/templates --out ../../app/hello-world/dist --css external --plugin=fast
esbuild ../../app/hello-world/src/index.ts --bundle --outfile=../../app/hello-world/dist/index.js --format=esm
```

## Usage

```bash
# hello-world
pnpm start ../../app/hello-world/dist ../../app/hello-world/data/state.json --plugin=fast

# contact-book-manager
pnpm start ../../app/contact-book-manager/dist ../../app/contact-book-manager/data/state.json --plugin=fast
```

## CLI Arguments

| Argument | Description |
|---|---|
| `dist-dir` | **(required)** Path to the app's `dist/` directory containing `protocol.bin` and CSS/JS assets |
| `state.json` | **(required)** Path to the state JSON file |
| `--plugin=fast` | Enable FAST hydration plugin _(optional)_ |
