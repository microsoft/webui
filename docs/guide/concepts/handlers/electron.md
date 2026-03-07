# WebUI Electron Handler

WebUI apps can run as native desktop applications using Electron. The Electron integration uses the `webui-node` native addon to render pre-built protocols at startup, then serves the rendered HTML and static assets through a custom `webui://` protocol scheme — no HTTP server needed.

## How it Works

1. **Build phase** — `webui build --plugin=fast` compiles templates into `protocol.bin`. esbuild bundles the client JS.
2. **Startup** — Electron's main process loads the native addon (`webui-node`), reads `protocol.bin` + `state.json`, and calls `addon.render()` to produce the full SSR HTML.
3. **Custom protocol** — A `webui://` protocol scheme is registered. When Electron loads `webui://app/`, it serves the rendered HTML. CSS and JS assets are served from the app's `dist/` directory.
4. **Hydration** — The client JS bundle hydrates the SSR output, attaching event listeners and enabling interactivity — same as in a browser.

## Usage

```bash
# 1. Build the native addon
cargo build -p webui-node --release

# 2. Build a WebUI app (e.g., contact-book-manager)
cd examples/app/contact-book-manager
npm run build

# 3. Run it in Electron
cd examples/integration/electron
npm run build
npx electron dist/main.js ../../app/contact-book-manager/dist ../../app/contact-book-manager/data/state.json --plugin=fast
```

## CLI Arguments

| Argument | Description | Default |
|----------|-------------|---------|
| `dist-dir` | Path to app's `dist/` directory with `protocol.bin` and assets | `../../app/hello-world/dist` |
| `state.json` | Path to state JSON file | `../../app/hello-world/data/state.json` |
| `--plugin=fast` | Enable FAST hydration plugin | None |

## Custom Titlebar

The integration uses `titleBarStyle: 'hidden'` with `titleBarOverlay` for a frameless native look. The app's header component uses `-webkit-app-region: drag` to act as the drag handle. Interactive elements within the header use `-webkit-app-region: no-drag` to remain clickable.

## The `webui://` Protocol

Electron's custom protocol handler maps routes to content:

- `webui://app/` → SSR-rendered HTML
- `webui://app/*.css` → CSS files from the dist directory
- `webui://app/*.js` → JS bundles from the dist directory

This avoids the need for a local HTTP server and provides a clean, secure origin for the app.

## Example

A complete working example is available at [`examples/integration/electron/`](https://github.com/user/webui/tree/main/examples/integration/electron).

The [Contact Book Manager](https://github.com/user/webui/tree/main/examples/app/contact-book-manager) app demonstrates a full-featured WebUI application that works both in the browser (via `webui serve`) and as an Electron desktop app.

## Performance Notes

- The native addon renders the entire page synchronously at startup — no per-request overhead.
- Protocol data is loaded once and rendered once. The custom protocol handler serves pre-rendered HTML from memory.
- Client-side hydration runs identically to the browser — same JS bundle, same FAST-HTML components.
