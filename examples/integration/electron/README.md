# WebUI Electron Integration

Wraps any pre-built WebUI app in a frameless Electron desktop window using the `@microsoft/webui` package.

## Prerequisites

1. Build the native addon:

```bash
cargo build -p microsoft-webui-node --release
```

2. Build the `@microsoft/webui` package:

```bash
pnpm --filter @microsoft/webui build
```

3. Install workspace dependencies:

```bash
pnpm install
```

4. Build a WebUI app (e.g. contact-book-manager):

```bash
cargo run -p microsoft-webui-cli -- build ../../app/contact-book-manager/src --out ../../app/contact-book-manager/dist --css link --plugin=webui
esbuild ../../app/contact-book-manager/src/index.ts --bundle --outfile=../../app/contact-book-manager/dist/index.js --format=esm
```

## Usage

```bash
# contact-book-manager (WebUI Framework)
pnpm start ../../app/contact-book-manager/dist ../../app/contact-book-manager/data/state.json --plugin=webui
```

## CLI Arguments

| Argument | Description |
|---|---|
| `dist-dir` | **(required)** Path to the app's `dist/` directory containing `protocol.bin` and CSS/JS assets |
| `state.json` | **(required)** Path to the state JSON file |
| `--plugin=<name>` | Hydration plugin identifier (see the WebUI documentation for available plugins) _(optional)_ |
