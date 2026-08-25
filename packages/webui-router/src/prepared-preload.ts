// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import {
  prependBasePath,
  stripBaseFromPathname,
} from './navigation-path.js';
import { pointerPreloadPath } from './preload-path.js';

const PRELOAD_TTL_MS = 5_000;
const MAX_PRELOAD_BYTES = 2_000_000;
const INVENTORY_FIELD = '"inventory":"';

export interface RawPreloadedPartial {
  readonly controller: AbortController;
  readonly inventory: string;
  readonly response: Response;
  readonly timestamp: number;
}

export interface PreparedRoutePreloadOptions {
  readonly excludePaths?: readonly string[];
  /** Begin deferred application work after a valid internal link intent. */
  readonly onIntent?: () => void | Promise<unknown>;
}

/** One bounded pre-hydration route prefetch that can be adopted by the router. */
export class PreparedRoutePreload {
  private readonly basePath =
    document.querySelector('base')?.getAttribute('href')?.replace(/\/+$/, '') ?? '';
  private readonly currentRequestPath =
    (stripBaseFromPathname(location.pathname, this.basePath) + location.search) || '/';
  private readonly excludePaths: readonly string[];
  private readonly onIntent:
    | (() => void | Promise<unknown>)
    | undefined;
  private controller: AbortController | null = null;
  private path: string | null = null;
  private adoptedPath: string | null = null;
  private adoptedController: AbortController | null = null;
  private pending: Promise<RawPreloadedPartial | null> | null = null;
  private expiryTimer: ReturnType<typeof setTimeout> | null = null;
  private listening = true;

  constructor(options: PreparedRoutePreloadOptions = {}) {
    this.excludePaths = options.excludePaths ?? [];
    this.onIntent = options.onIntent;
    document.addEventListener('pointermove', this.onPointerMove);
  }

  has(requestPath: string): boolean {
    return (
      (this.path === requestPath && this.pending !== null)
      || this.adoptedPath === requestPath
    );
  }

  detach(): void {
    if (!this.listening) return;
    this.listening = false;
    document.removeEventListener('pointermove', this.onPointerMove);
  }

  release(requestPath: string): void {
    if (this.adoptedPath !== requestPath) return;
    this.adoptedPath = null;
    this.adoptedController = null;
  }

  destroy(): void {
    this.detach();
    this.controller?.abort();
    this.adoptedController?.abort();
    this.clearExpiry();
    this.controller = null;
    this.path = null;
    this.adoptedPath = null;
    this.adoptedController = null;
    this.pending = null;
  }

  async take(
    requestPath: string,
    inventory: string,
    signal?: AbortSignal,
  ): Promise<RawPreloadedPartial | null> {
    if (this.path !== requestPath || !this.pending) return null;
    const pending = this.pending;
    const controller = this.controller;
    this.adoptedController = controller;
    this.controller = null;
    this.path = null;
    this.pending = null;
    this.adoptedPath = requestPath;
    this.clearExpiry();
    this.expiryTimer = setTimeout(
      () => this.expireAdopted(requestPath),
      PRELOAD_TTL_MS,
    );
    const result = signal
      ? await withAbort(pending, signal, () => controller?.abort())
      : await pending;
    this.clearExpiry();
    if (
      !result
      || result.inventory !== inventory
      || Date.now() - result.timestamp > PRELOAD_TTL_MS
    ) {
      result?.controller.abort();
      this.release(requestPath);
      return null;
    }
    this.adoptedController = result.controller;
    return result;
  }

  private readonly onPointerMove = (event: PointerEvent): void => {
    const requestPath = pointerPreloadPath(
      event,
      this.basePath,
      this.excludePaths,
    );
    if (
      !requestPath
      || requestPath === this.currentRequestPath
      || requestPath === this.path
    ) {
      return;
    }

    this.controller?.abort();
    this.clearExpiry();
    const controller = new AbortController();
    const inventory = hydrationInventory();
    this.controller = controller;
    this.path = requestPath;
    const pending = fetchRawPartial(
      prependBasePath(requestPath, this.basePath),
      inventory,
      controller,
    );
    this.pending = pending;
    this.expiryTimer = setTimeout(() => this.expirePending(), PRELOAD_TTL_MS);
    try {
      const intent = this.onIntent?.();
      if (intent) void intent.catch(() => this.destroy());
    } catch {
      this.destroy();
      return;
    }
    void pending.then((result) => {
      if (!result && this.pending === pending) {
        this.clearExpiry();
        this.controller = null;
        this.path = null;
        this.pending = null;
      } else if (result && this.pending === pending) {
        this.clearExpiry();
        this.expiryTimer = setTimeout(
          () => this.expirePending(),
          PRELOAD_TTL_MS,
        );
      }
    });
  };

  private clearExpiry(): void {
    if (this.expiryTimer === null) return;
    clearTimeout(this.expiryTimer);
    this.expiryTimer = null;
  }

  private expirePending(): void {
    this.expiryTimer = null;
    this.controller?.abort();
    this.controller = null;
    this.path = null;
    this.pending = null;
  }

  private expireAdopted(requestPath: string): void {
    this.expiryTimer = null;
    if (this.adoptedPath !== requestPath) return;
    this.adoptedController?.abort();
    this.adoptedController = null;
    this.adoptedPath = null;
  }
}

function hydrationInventory(): string {
  if (window.__webui?.inventory) return window.__webui.inventory;
  const text = document.getElementById('webui-data')?.textContent;
  if (!text) return '';
  const start = text.indexOf(INVENTORY_FIELD);
  if (start < 0) return '';
  const valueStart = start + INVENTORY_FIELD.length;
  const valueEnd = text.indexOf('"', valueStart);
  if (valueEnd < valueStart) return '';
  const inventory = text.slice(valueStart, valueEnd);
  for (let i = 0; i < inventory.length; i++) {
    const code = inventory.charCodeAt(i);
    if (
      !(
        (code >= 48 && code <= 57)
        || (code >= 65 && code <= 70)
        || (code >= 97 && code <= 102)
      )
    ) {
      return '';
    }
  }
  return inventory;
}

async function fetchRawPartial(
  path: string,
  inventory: string,
  controller: AbortController,
): Promise<RawPreloadedPartial | null> {
  const signal = controller.signal;
  try {
    const headers: Record<string, string> = {
      Accept: 'application/x-ndjson, application/json',
    };
    if (inventory) headers['X-WebUI-Inventory'] = inventory;
    const response = await fetch(path, { headers, signal });
    const contentType = response.headers.get('content-type') ?? '';
    if (
      !response.ok
      || (!contentType.includes('json') && !contentType.includes('ndjson'))
    ) {
      controller.abort();
      return null;
    }
    const prepared = contentType.includes('ndjson')
      ? await prepareNdjsonResponse(response, signal)
      : await prepareJsonResponse(response, signal);
    return prepared
      ? {
          controller,
          inventory,
          response: prepared,
          timestamp: Date.now(),
        }
      : null;
  } catch {
    return null;
  }
}

async function prepareJsonResponse(
  response: Response,
  signal: AbortSignal,
): Promise<Response | null> {
  const reader = response.body?.getReader();
  if (!reader) return copyResponse(response, null);
  const chunks: Uint8Array[] = [];
  let total = 0;
  while (true) {
    const { done, value } = await reader.read();
    if (signal.aborted) {
      await reader.cancel();
      return null;
    }
    if (done) break;
    total += value.byteLength;
    if (total > MAX_PRELOAD_BYTES) {
      await reader.cancel();
      return null;
    }
    chunks.push(value);
  }
  const bytes = new Uint8Array(total);
  let offset = 0;
  for (let i = 0; i < chunks.length; i++) {
    bytes.set(chunks[i], offset);
    offset += chunks[i].byteLength;
  }
  return copyResponse(response, new Uint8Array(bytes).buffer);
}

async function prepareNdjsonResponse(
  response: Response,
  signal: AbortSignal,
): Promise<Response | null> {
  const reader = response.body?.getReader();
  if (!reader) return copyResponse(response, null);
  const buffered: Uint8Array[] = [];
  let total = 0;
  let ended = false;
  let foundLine = false;
  while (!foundLine && !ended) {
    const { done, value } = await reader.read();
    if (signal.aborted) {
      await reader.cancel();
      return null;
    }
    ended = done;
    if (!value) continue;
    total += value.byteLength;
    if (total > MAX_PRELOAD_BYTES) {
      await reader.cancel();
      return null;
    }
    buffered.push(value);
    for (let i = 0; i < value.length; i++) {
      if (value[i] === 10) {
        foundLine = true;
        break;
      }
    }
  }
  if (buffered.length === 0) return null;

  const stream = new ReadableStream<Uint8Array>({
    start(controller) {
      for (let i = 0; i < buffered.length; i++) {
        controller.enqueue(buffered[i]);
      }
      if (ended) controller.close();
    },
    async pull(controller) {
      const { done, value } = await reader.read();
      if (done) {
        controller.close();
        return;
      }
      total += value.byteLength;
      if (total > MAX_PRELOAD_BYTES) {
        await reader.cancel();
        controller.error(new RangeError('Prepared route partial exceeded 2 MB'));
        return;
      }
      controller.enqueue(value);
    },
    async cancel(reason) {
      await reader.cancel(reason);
    },
  });
  return copyResponse(response, stream);
}

function copyResponse(response: Response, body: BodyInit | null): Response {
  return new Response(body, {
    headers: response.headers,
    status: response.status,
    statusText: response.statusText,
  });
}

function withAbort<T>(
  pending: Promise<T>,
  signal: AbortSignal,
  abort: () => void,
): Promise<T | null> {
  if (signal.aborted) {
    abort();
    return Promise.resolve(null);
  }
  return new Promise((resolve) => {
    let settled = false;
    const onAbort = () => {
      if (settled) return;
      settled = true;
      abort();
      resolve(null);
    };
    signal.addEventListener('abort', onAbort, { once: true });
    void pending.then((value) => {
      if (settled) return;
      settled = true;
      signal.removeEventListener('abort', onAbort);
      resolve(value);
    });
  });
}

export function prepareRoutePreload(
  options?: PreparedRoutePreloadOptions,
): PreparedRoutePreload {
  return new PreparedRoutePreload(options);
}
