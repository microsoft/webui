// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/// <reference lib="webworker" />

import { API_CHUNKS, sanitizePayload, type ApiChunk, type ApiPayload } from './payload.js';
import initWasm, { render } from './wasm/handler/webui_wasm_handler.js';

declare const self: ServiceWorkerGlobalScope;

const baseUrl = new URL("./", self.location.href);
const protocolUrl = new URL("protocol.bin", baseUrl);
const themeCssUrl = new URL("theme.css", baseUrl);
const encoder = new TextEncoder();
let wasmReady: Promise<unknown> | undefined;
let protocolReady: Promise<Uint8Array> | undefined;
let themeCssReady: Promise<string> | undefined;

self.addEventListener("install", () => {
  self.skipWaiting();
});

self.addEventListener("activate", (event: ExtendableEvent) => {
  event.waitUntil(self.clients.claim());
});

self.addEventListener("fetch", (event: FetchEvent) => {
  if (event.request.mode === 'navigate') {
    event.respondWith(streamNavigation());
  }
});

async function streamNavigation(): Promise<Response> {
  const stream = new ReadableStream<Uint8Array>({
    async start(controller) {
      try {
        await streamHtml(controller);
      } catch (error) {
        controller.enqueue(encode(renderError(error)));
      } finally {
        controller.close();
      }
    },
  });

  return new Response(stream, {
    headers: {
      'Content-Type': 'text/html; charset=utf-8',
      'Cache-Control': 'no-store',
    },
  });
}

async function streamHtml(controller: ReadableStreamDefaultController<Uint8Array>): Promise<void> {
  const themeCss = await loadThemeCss();
  controller.enqueue(encode(documentStart(themeCss)));

  const protocol = await loadProtocol();
  controller.enqueue(encode("<!-- protocol-ready -->\n"));

  await streamChunksAsReady(controller, protocol);
  controller.enqueue(encode(documentEnd()));
}

async function loadThemeCss(): Promise<string> {
  if (!themeCssReady) {
    themeCssReady = fetch(themeCssUrl, { cache: "no-cache" })
      .then((response) => {
        if (!response.ok) {
          throw new Error(`Failed to load ${themeCssUrl.pathname}: ${response.status}`);
        }
        return response.text();
      })
      .catch((error) => {
        themeCssReady = undefined;
        throw error;
      });
  }
  return themeCssReady;
}

async function loadProtocol(): Promise<Uint8Array> {
  if (!protocolReady) {
    protocolReady = (async () => {
      await loadWasm();
      const response = await fetch(protocolUrl, { cache: "no-cache" });
      if (!response.ok) {
        throw new Error(`Failed to load ${protocolUrl.pathname}: ${response.status}`);
      }
      return new Uint8Array(await response.arrayBuffer());
    })().catch((error) => {
      protocolReady = undefined;
      throw error;
    });
  }
  return protocolReady;
}

function loadWasm(): Promise<unknown> {
  if (!wasmReady) {
    const ready = initWasm().catch((error: unknown) => {
      wasmReady = undefined;
      throw error;
    });
    wasmReady = ready;
    return ready;
  }
  return wasmReady;
}

async function streamChunksAsReady(
  controller: ReadableStreamDefaultController<Uint8Array>,
  protocol: Uint8Array,
): Promise<void> {
  const pending = API_CHUNKS.map((chunk) =>
    fetchChunk(chunk).then((payload) => ({ chunk, payload })),
  );

  while (pending.length > 0) {
    const indexed = pending.map((promise, index) =>
      promise.then((result) => ({ index, result })),
    );
    const { index, result } = await Promise.race(indexed);
    pending.splice(index, 1);

    controller.enqueue(
      encode(`<div class="stream-chunk" data-chunk="${result.chunk.label}">\n`),
    );
    render(
      protocol,
      JSON.stringify(result.payload.state),
      (chunk: string) => controller.enqueue(encode(chunk)),
      {
        entry: result.payload.entry,
        requestPath: '/',
        plugin: 'webui',
      },
    );
    controller.enqueue(encode("\n</div>\n"));
  }
}

async function fetchChunk(chunk: ApiChunk): Promise<ApiPayload> {
  const url = new URL(chunk.path, baseUrl);
  const response = await fetch(url, {
    headers: { Accept: 'application/json' },
    cache: 'no-cache',
  });
  if (!response.ok) {
    throw new Error(`Failed to load ${url.pathname}: ${response.status}`);
  }

  const payload = await response.json();
  const sanitized = sanitizePayload(payload, chunk.path, baseUrl);
  await delay(sanitized.delayMs);
  return sanitized;
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, ms);
  });
}

function encode(value: string): Uint8Array {
  return encoder.encode(value);
}

function documentStart(themeCss: string): string {
  return `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>WebUI Service Worker Streaming</title>
  <style>${themeCss}</style>
</head>
<body>
  <main class="page">
    <div class="stream-note">Streaming from service worker + WebUI WASM handler</div>
`;
}

function documentEnd(): string {
  return `  </main>
</body>
</html>
`;
}

function renderError(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  return `<section class="card error-card"><h1>Render failed</h1><p>${escapeHtml(message)}</p></section>`;
}

function escapeHtml(value: string): string {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}
