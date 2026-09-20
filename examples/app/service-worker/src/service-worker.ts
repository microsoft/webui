// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/// <reference lib="webworker" />

import { API_CHUNKS, sanitizePayload, type ApiChunk, type ApiPayload } from './payload.js';
import initWasm, {
  type BoundaryDescriptor,
  Protocol,
  type StreamStep,
} from './wasm/handler/webui_wasm_handler.js';

declare const self: ServiceWorkerGlobalScope;

const baseUrl = new URL("./", self.location.href);
const protocolUrl = new URL("protocol.bin", baseUrl);
const themeCssUrl = new URL("theme.css", baseUrl);
const encoder = new TextEncoder();
let wasmReady: Promise<unknown> | undefined;
let protocolReady: Promise<Protocol> | undefined;
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
  const [themeCss, protocol] = await Promise.all([loadThemeCss(), loadProtocol()]);
  const session = protocol.streamResponse('index.html', '/', {
    headInject: `<style>${themeCss}</style>`,
  });
  const pending = new Map<
    string,
    Promise<{ chunk: ApiChunk; payload: ApiPayload }>
  >(
    API_CHUNKS.map((chunk) => [
      chunk.label,
      fetchChunk(chunk).then((payload) => ({ chunk, payload })),
    ]),
  );

  let step = session.start('{}');
  controller.enqueue(step.bytes);
  while (!step.done) {
    if (!step.boundary) {
      step = session.advance();
      controller.enqueue(step.bytes);
      continue;
    }
    const boundary = pendingBoundary(step);
    const result = await pending.get(boundary.name);
    if (!result) {
      throw new Error(
        `Unexpected streaming boundary ${boundary.owner}/${boundary.name}`,
      );
    }
    pending.delete(boundary.name);
    validateBoundaryPayload(boundary, result.chunk, result.payload);
    step = session.resume(
      boundary.instanceId,
      JSON.stringify(result.payload.state),
    );
    controller.enqueue(step.bytes);
  }
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

async function loadProtocol(): Promise<Protocol> {
  if (!protocolReady) {
    protocolReady = (async () => {
      await loadWasm();
      const response = await fetch(protocolUrl, { cache: "no-cache" });
      if (!response.ok) {
        throw new Error(`Failed to load ${protocolUrl.pathname}: ${response.status}`);
      }
      const bytes = new Uint8Array(await response.arrayBuffer());
      return new Protocol(bytes, 'webui');
    })().catch((error) => {
      protocolReady = undefined;
      throw error;
    });
  }
  return protocolReady;
}

function loadWasm(): Promise<unknown> {
  if (!wasmReady) {
    const ready = Promise.resolve(initWasm()).catch((error: unknown) => {
      wasmReady = undefined;
      throw error;
    });
    wasmReady = ready;
    return ready;
  }
  return wasmReady;
}

function pendingBoundary(step: StreamStep): BoundaryDescriptor {
  if (step.done || !step.boundary) {
    throw new Error('Streaming session returned no pending boundary');
  }
  return step.boundary;
}

function validateBoundaryPayload(
  boundary: BoundaryDescriptor,
  chunk: ApiChunk,
  payload: ApiPayload,
): void {
  if (boundary.owner !== 'index.html' || boundary.name !== chunk.label) {
    throw new Error(
      `Expected index.html/${chunk.label}, received ${boundary.owner}/${boundary.name}`,
    );
  }
  if (payload.entry !== `${chunk.label}-panel`) {
    throw new Error(
      `API payload for ${chunk.label} targets unexpected entry ${payload.entry}`,
    );
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
