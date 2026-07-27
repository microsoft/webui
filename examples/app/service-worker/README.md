# WebUI service worker integration

This example shows a static/CDN-style deployment where a service worker uses the
handler-only WebUI WASM bundle to render HTML in the browser.

The browser loads only static assets:

- `protocol.bin` generated at build time
- `webui_wasm_handler.js` plus its `.wasm` payload
- public JSON files under `/api/`
- a service worker that streams the navigation response

No application server is required. The service worker fetches public API state,
constructs one `webui_wasm_handler.Protocol`, renders matching fragments with
`Protocol.renderStream()`, and enqueues each section into a `ReadableStream` as
soon as that API response resolves.

Because the example renders public API data, the service worker validates URL
fields before calling the handler. Keep that boundary in copied code so
untrusted API state cannot inject unsafe link schemes.

## Build

From the repository root:

```bash
pnpm --filter service-worker-example build
```

The build step:

1. Compiles `src/` to `public/protocol.bin` with light DOM output.
2. Bundles the TypeScript helper scripts into `dist-scripts/`.
3. Copies the handler-only WASM bundle into `public/wasm/handler/`.
4. Type-checks the browser, worker, test, and script TypeScript.
5. Injects resolved theme CSS into `public/index.html` from `protocol.bin`
   token metadata.
6. Bundles `src/app.ts`, `src/payload.ts`, and `src/service-worker.ts` into
   `public/`.
7. Renders every sample API payload once through the WASM handler.

The streamed UI uses normal WebUI component files: every `src/*.html` fragment
has a paired `src/*.css` file. The app is built with `--css style --dom light`,
so component CSS is embedded in the rendered fragments and no standalone
`public/styles.css` file is needed.

For design consistency, the service worker uses
`@microsoft/webui-examples-theme`, the same shared token package used by
`webui serve --theme=@microsoft/webui-examples-theme`. Because this app renders
in a static service worker instead of `webui serve`, `scripts/inject-theme.ts`
reads `public/protocol.bin`, asks the WASM handler for the protocol token list,
and writes trusted token declarations into `public/index.html` and
`public/theme.css` at build time.
Runtime API state never carries theme CSS.

If the handler WASM bundle has not been generated yet, the copy helper runs:

```bash
cargo xtask build-wasm
```

## Run

```bash
cd examples/app/service-worker
pnpm start
```

Open `http://localhost:4175/`. The first load installs the service worker and
reloads. The service worker then intercepts the navigation and streams HTML
sections as the API JSON files complete.

## Test

```bash
pnpm --filter service-worker-example test
```

The Playwright smoke test verifies that the page is controlled by the service
worker, renders all WebUI chunks, and receives the chunks in async completion
order rather than source order. Each chunk is wrapped with a `data-chunk`
marker so the stream order is visible and easy to assert.

## Why this matters

This pattern is useful when HTML shell assets live on a CDN and all data comes
from public APIs. The serverless edge path can be:

1. CDN serves static files.
2. Browser service worker loads `protocol.bin`.
3. Public APIs return JSON state.
4. WebUI WASM handler renders HTML chunks locally.
5. The service worker streams the response to the page.

## Source layout

| Path | Purpose |
|------|---------|
| `src/*.html` | WebUI fragments compiled into `public/protocol.bin` |
| `src/*.css` | Component styles embedded into rendered light-DOM fragments |
| `src/bootstrap.html` | Static first-load page stamped into `public/index.html` with build-time theme declarations |
| `src/*.ts` | TypeScript browser runtime, service worker, and payload validation |
| `public/api/*.json` | Public API state payloads used by each fragment |
| `scripts/*.ts` | Build helpers for copying WASM, injecting theme CSS, and smoke-rendering payloads |
| `tsconfig*.json` | Strict TypeScript configs for browser/runtime and worker contexts |
