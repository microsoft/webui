# WebUI Electron Integration

Wraps any pre-built WebUI app in a frameless Electron desktop window using the `webui-node` native addon.

## Prerequisites

1. Build the native addon:

```bash
cargo build -p webui-node --release
```

2. Build a WebUI app (e.g. hello-world):

```bash
cargo run -p webui-cli -- build ../../app/hello-world/templates --out ../../app/hello-world/dist --css external --plugin=fast
esbuild ../../app/hello-world/src/index.ts --bundle --outfile=../../app/hello-world/dist/index.js --format=esm
```

## Usage

Run with the default hello-world app:

```bash
pnpm start
```

Or point to any other WebUI app:

```bash
# hello-world
electron dist/main.js ../../app/hello-world/dist ../../app/hello-world/data/state.json --plugin=fast

# contact-book-manager
electron dist/main.js ../../app/contact-book-manager/dist ../../app/contact-book-manager/data/state.json --plugin=fast
```

## CLI Arguments

| Argument | Description | Default |
|---|---|---|
| `dist-dir` | Path to the app's `dist/` directory containing `protocol.bin` and CSS/JS assets | `../../app/hello-world/dist` |
| `state.json` | Path to the state JSON file | `../../app/hello-world/data/state.json` |
| `--plugin=fast` | Enable FAST hydration plugin | _(disabled)_ |
