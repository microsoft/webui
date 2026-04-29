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

4. Build a WebUI app (e.g. todo-fast):

```bash
cargo run -p microsoft-webui-cli -- build ../../app/todo-fast/src --out ../../app/todo-fast/dist --css link --plugin=fast-v3
esbuild ../../app/todo-fast/src/index.ts --bundle --outfile=../../app/todo-fast/dist/index.js --format=esm
```

## Usage

```bash
# todo-fast (@microsoft/fast-element 3.x)
pnpm start ../../app/todo-fast/dist ../../app/todo-fast/data/state.json --plugin=fast-v3

# contact-book-manager (WebUI Framework)
pnpm start ../../app/contact-book-manager/dist ../../app/contact-book-manager/data/state.json --plugin=webui
```

## CLI Arguments

| Argument | Description |
|---|---|
| `dist-dir` | **(required)** Path to the app's `dist/` directory containing `protocol.bin` and CSS/JS assets |
| `state.json` | **(required)** Path to the state JSON file |
| `--plugin=<name>` | Enable a hydration plugin: `webui`, `fast-v3`, deprecated `fast-v2`, or deprecated `fast` _(optional)_ |

Deprecated @microsoft/fast-element 2.x compatibility remains available with `--plugin=fast-v2` or the `--plugin=fast` alias.
