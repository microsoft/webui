---
title: "Rendering in a service worker"
description: "Run the handler-only WASM example, understand its bootstrap load, and separate browser rendering from native server-side rendering."
---

<a id="serverless-architecture"></a>

# Rendering in a Service Worker

The [service-worker example](https://github.com/microsoft/webui/tree/main/examples/app/service-worker)
renders HTML in the browser using WebUI's handler-only WASM bundle. A static
host serves the compiled UI and bootstrap page; APIs provide public JSON state.

This is an alternative deployment model, **not ordinary native SSR**.
Normally a WebUI server loads `protocol.bin` and sends HTML to the browser.
In this example the browser downloads the protocol and WASM and pays their
transfer, initialization, rendering, and memory costs.

## Run it

From a source checkout with the workspace dependencies installed:

```bash
pnpm --filter service-worker-example build
cd examples/app/service-worker
pnpm start
```

Open `http://localhost:4175/`. The first, uncontrolled load serves a static
bootstrap page. It registers the service worker and reloads after activation;
only a controlled navigation receives worker-rendered HTML. Deployment requires
a service-worker-capable browser and a secure context (HTTPS, or localhost for
development).

## Cache UI separately from data

The compiled UI, WASM handler, and theme assets can be cached as a versioned
set. A worker can reuse that UI while fetching fresh application state, then
stream rendered HTML fragments as each boundary's data becomes ready.

Your app chooses cache storage, invalidation, and offline fallback behavior.
The example below reuses loaded resources during the worker's lifetime and
revalidates network fetches; it does not install a persistent offline cache.

## Sequence

| Step | What happens |
| --- | --- |
| First load | The static host serves `index.html` and its registration script. No worker render occurs yet. |
| Activation | The worker claims clients; the bootstrap script reloads the page. |
| Controlled navigation | The worker loads the handler WASM, `protocol.bin`, and theme CSS, reusing them during its lifetime. |
| State loading | Public API requests start in parallel. |
| Rendering | A host-driven `streamResponse()` session resumes each boundary when its data is ready, in document order. |
| Delivery | The worker enqueues returned bytes into a `ReadableStream` response for the browser to parse. |

Later boundaries do not overtake an earlier unresolved boundary. A restarted
worker may need to reload its resources. The example is not an offline cache.

## Why this is different

| Native WebUI SSR | Service-worker rendering |
|-----------------|--------------------------------|
| The server loads the compiled protocol | The browser loads the compiled protocol and WASM |
| The first document can contain rendered application HTML | The first uncontrolled load is a bootstrap page |
| The host renders with server-provided state | Public APIs provide state for browser-side rendering |
| Server capacity and transport affect delivery | Device capacity, worker startup, and API latency also affect delivery |

Consider this for a browser application backed by public data where a bootstrap
load is acceptable. Prefer native SSR when the initial response needs complete
HTML without JavaScript. Browser-side rendering does not move authorization
into the worker or make API data private.

## Local code map

| Path | What to inspect |
|------|-----------------|
| `examples/app/service-worker/src/*.html` | WebUI fragments compiled into `protocol.bin` |
| `examples/app/service-worker/src/*.css` | Component CSS embedded into streamed light-DOM fragments |
| `examples/app/service-worker/src/bootstrap.html` | Static first-load page stamped with build-time theme declarations |
| `examples/app/service-worker/src/service-worker.ts` | Navigation interception, state fetches, progressive WASM session, `ReadableStream` writes |
| `examples/app/service-worker/src/payload.ts` | Public API payload validation and URL sanitization |
| `examples/app/service-worker/scripts/inject-theme.ts` | Build-time equivalent of `webui serve --theme` token injection |
| `examples/app/service-worker/public/api/*.json` | Demo public state APIs |
| `examples/app/service-worker/tests/service-worker.spec.ts` | Browser test for service worker control and stream order |

## Key design choices

1. **Handler-only WASM.** The browser uses `webui_wasm_handler`, not the parser
   bundle. Parsing happens at build time.
2. **Static UI assets.** A CDN can serve the protocol and WASM, but configure
   versioning and cache headers together. The example revalidates protocol and
   theme fetches; it does not install an offline asset cache.
3. **Dynamic state only.** API endpoints return JSON state. They do not build
   HTML and do not need a JavaScript rendering runtime.
4. **Host-paced streaming.** Reuse `Protocol`, then create a `streamResponse`
   session per navigation. Call `start`, `resume`, and `advance`, writing each
   step's bytes to the response. See the [WASM API](/guide/integrations/wasm).
5. **CSS delivery.** The example builds with `--css style` and supplies the
   shared theme through the session's `headInject` option.
6. **Shared theme tokens.** `scripts/inject-theme.ts` mirrors
   `webui serve --theme=@microsoft/webui-examples-theme` at build time. It reads
   `protocol.bin`, gets the protocol token list, and writes trusted token
   declarations into `public/index.html` and `public/theme.css`.
7. **State validation boundary.** Public API state is validated before render.
   URL-bearing fields allow only `https:` or same-origin links.

## Test it

```bash
pnpm --filter service-worker-example test
```

The Playwright test verifies service worker control, visible rendered chunks,
async chunk ordering, theme declaration streaming, no standalone `styles.css`
request, and no browser console errors.
