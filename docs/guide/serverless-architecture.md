# Serverless Architecture

The [`examples/app/service-worker`](https://github.com/microsoft/webui/tree/main/examples/app/service-worker)
example demonstrates a high-value WebUI architecture: cache the UI on a CDN and
pay dynamic compute cost only for state.

The expensive part of SSR is usually rebuilding the same page shell over and
over. WebUI changes that split. The HTML structure and component styles compile
once into `protocol.bin`, which can be cached next to the handler-only WASM
bundle. On each navigation, the browser service worker fetches only public JSON
state, renders fragments locally with the lightweight WASM handler, and streams
HTML into the page as each API response arrives.

That means the origin or serverless function does not render HTML. It only
serves state. The UI, protocol, theme, and WASM handler can all live behind CDN
cache.

## Sequence

```text
Browser        Service Worker        CDN / Edge Cache        Public APIs        webui_wasm_handler
   |                 |                       |                    |                    |
   | GET /           |                       |                    |                    |
   |---------------------------------------->|                    |                    |
   | index.html + app.js                    |                    |                    |
   |<----------------------------------------|                    |                    |
   | register()      |                       |                    |                    |
   |---------------->|                       |                    |                    |
   | claim clients   |                       |                    |                    |
   |<----------------|                       |                    |                    |
   | navigate /      |                       |                    |                    |
   |---------------->|                       |                    |                    |
   |                 | GET /protocol.bin     |                    |                    |
   |                 |---------------------->|                    |                    |
   |                 | cached protocol       |                    |                    |
   |                 |<----------------------|                    |                    |
   |                 | GET /wasm/handler/... |                    |                    |
   |                 |---------------------->|                    |                    |
   |                 | cached handler WASM   |                    |                    |
   |                 |<----------------------|                    |                    |
   |                 | GET /theme.css        |                    |                    |
   |                 |---------------------->|                    |                    |
   |                 | cached theme CSS      |                    |                    |
   |                 |<----------------------|                    |                    |
   |                 | GET /api/*.json       |------------------->|                    |
   |                 | JSON state            |<-------------------|                    |
   |                 | render(state chunk)   |                    |------------------->|
   |                 | onChunk(rendered HTML)|                    |<-------------------|
   | stream fragment |                       |                    |                    |
   |<----------------|                       |                    |                    |
```

## Why this is different

| Traditional SSR | WebUI service worker streaming |
|-----------------|--------------------------------|
| Server renders HTML per request | CDN serves cached protocol and WASM |
| Dynamic compute repeats the UI shell | Dynamic compute returns only JSON state |
| First byte waits for server render | Service worker can stream fragments as APIs resolve |
| App server owns HTML rendering | Browser owns rendering through the handler-only WASM bundle |

This architecture is especially useful for public-data sites, dashboards, docs
portals, marketing pages with personalized sections, and any serverless app
where the UI changes far less often than the state.

## Local code map

| Path | What to inspect |
|------|-----------------|
| `examples/app/service-worker/src/*.html` | WebUI fragments compiled into `protocol.bin` |
| `examples/app/service-worker/src/*.css` | Component CSS embedded into streamed light-DOM fragments |
| `examples/app/service-worker/src/bootstrap.html` | Static first-load page stamped with build-time theme declarations |
| `examples/app/service-worker/src/service-worker.ts` | Navigation interception, state fetches, WASM render callback, `ReadableStream` writes |
| `examples/app/service-worker/src/payload.ts` | Public API payload validation and URL sanitization |
| `examples/app/service-worker/scripts/inject-theme.ts` | Build-time equivalent of `webui serve --theme` token injection |
| `examples/app/service-worker/public/api/*.json` | Demo public state APIs |
| `examples/app/service-worker/tests/service-worker.spec.ts` | Browser test for service worker control and stream order |

## Key design choices

1. **Handler-only WASM.** The browser uses `webui_wasm_handler`, not the parser
   bundle. Parsing happens at build time.
2. **Cached UI protocol.** `protocol.bin` is a static artifact. It can be served
   with long-lived CDN caching and invalidated only when UI changes.
3. **Dynamic state only.** API endpoints return JSON state. They do not build
   HTML and do not need a JavaScript rendering runtime.
4. **Callback streaming.** Construct `Protocol` once from the cached bytes,
   then call `protocol.renderStream(stateJson, onChunk, options)`. Handler
   writes are coalesced around a 16 KiB target before `onChunk(html)`, and the
   service worker writes those chunks directly to a `ReadableStream`.
5. **Light DOM fragments.** The example builds with `--dom light` because the
   service worker appends independent fragments into a single document stream.
6. **Shared theme tokens.** `scripts/inject-theme.ts` mirrors
   `webui serve --theme=@microsoft/webui-examples-theme` at build time. It reads
   `protocol.bin`, gets the protocol token list, and writes trusted token
   declarations into `public/index.html` and `public/theme.css`.
7. **State validation boundary.** Public API state is validated before render.
   URL-bearing fields allow only `https:` or same-origin links.

## Run it

```bash
pnpm --filter service-worker-example build
cd examples/app/service-worker
pnpm start
```

Open `http://localhost:4175/`. The first load installs the service worker and
reloads. The next navigation is streamed by the service worker.

## Test it

```bash
pnpm --filter service-worker-example test
```

The Playwright test verifies service worker control, visible rendered chunks,
async chunk ordering, theme declaration streaming, no standalone `styles.css`
request, and no browser console errors.
