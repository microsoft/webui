---
title: "Electron snapshot example"
description: "Run the repository's Electron viewer with a prebuilt protocol, native Node addon, and static assets served through webui URLs."
---

<a id="webui-electron-handler"></a>

# Electron

The repository's Electron example uses `@microsoft/webui` to render a prebuilt
protocol once in the main process. It serves that snapshot and static assets
through `webui://app/`, without an HTTP server. This is an example host, not a
separate Electron renderer package or a complete application backend.

## How it Works

1. **Build phase** - `webui build --plugin=<name>` compiles templates into `protocol.bin`. esbuild bundles the client JS.
2. **Startup** - The main process creates a `Protocol` from the native npm binding, reads JSON state, and collects `renderStream()` output into one HTML snapshot.
3. **Custom protocol** - A `webui://` protocol scheme is registered. When Electron loads `webui://app/`, it serves the rendered HTML. CSS and JS assets are served from the app's `dist/` directory.
4. **Hydration** - The client JS bundle hydrates the SSR output, attaching event listeners and enabling interactivity.

## Usage

From a source checkout with workspace dependencies installed:

```bash
# 1. Build the native addon
cargo build -p microsoft-webui-node --release
pnpm --filter @microsoft/webui build

# 2. Build an example app
pnpm --filter contact-book-manager build

# 3. Run it in Electron
cd examples/integration/electron
pnpm build
pnpm exec electron dist/main.js ../../app/contact-book-manager/dist ../../app/contact-book-manager/data/state.json --plugin=webui
```

## CLI Arguments

| Argument | Description | Default |
|----------|-------------|---------|
| `dist-dir` | Path to app's `dist/` directory with `protocol.bin` and assets | Required |
| `state.json` | Path to state JSON file | Required |
| `--plugin=<name>` | Hydration plugin matching the app's build (see [Plugins](/reference/plugins/)) | None |

## Custom Titlebar

The integration uses `titleBarStyle: 'hidden'` with `titleBarOverlay` for a
custom header beneath native controls. Mark draggable regions with
`-webkit-app-region: drag` and interactive children with
`-webkit-app-region: no-drag`.

## The `webui://` Protocol

Electron's custom protocol handler maps routes to content:

- `webui://app/` → SSR-rendered HTML
- `webui://app/*.css` → CSS files from the dist directory
- `webui://app/*.js` → JS bundles from the dist directory

Only `/` serves the rendered document. Server routing, API mutations, and
persistence require your own handlers. Use trusted build output and review
Electron's navigation and permission settings for your application; a custom
scheme is not itself a security boundary.

## Example

The host source is in
[`examples/integration/electron/`](https://github.com/microsoft/webui/tree/main/examples/integration/electron).

For a native system-webview host with route state and typed IPC, see
[Desktop Apps](/build/desktop).

## Performance Notes

- The native addon renders the page synchronously at startup, rather than on
  each navigation.
- The protocol is loaded once; the host serves the rendered snapshot from memory.
- The example uses the app's browser bundle for hydration. Electron still
  carries its own Chromium and Node runtime costs.
