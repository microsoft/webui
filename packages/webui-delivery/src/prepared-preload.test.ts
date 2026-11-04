// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import './browser-shim.js';
import { strict as assert } from 'node:assert';
import { afterEach, beforeEach, test } from 'node:test';
import { prepareRoutePreload } from './prepared-preload.js';

interface DocumentHooks {
  addEventListener(type: string, listener: EventListener): void;
  removeEventListener(type: string, listener: EventListener): void;
  querySelector(selector: string): Element | null;
}

const documentHooks = document as unknown as DocumentHooks;
const originalAdd = documentHooks.addEventListener;
const originalRemove = documentHooks.removeEventListener;
const originalQuery = documentHooks.querySelector;
const originalFetch = globalThis.fetch;
let pointerMove: ((event: PointerEvent) => void) | undefined;

beforeEach(() => {
  pointerMove = undefined;
  documentHooks.addEventListener = (type, listener) => {
    if (type === 'pointermove') {
      pointerMove = listener as (event: PointerEvent) => void;
    }
  };
  documentHooks.removeEventListener = (type, listener) => {
    if (type === 'pointermove' && pointerMove === listener) {
      pointerMove = undefined;
    }
  };
  documentHooks.querySelector = () => null;
  window.__webui = { inventory: '0a' };
});

afterEach(() => {
  documentHooks.addEventListener = originalAdd;
  documentHooks.removeEventListener = originalRemove;
  documentHooks.querySelector = originalQuery;
  globalThis.fetch = originalFetch;
  window.__webui = undefined;
});

test('deduplicates one link intent and hands off raw bytes once', async () => {
  let fetches = 0;
  globalThis.fetch = async () => {
    fetches++;
    return jsonResponse({ path: '/next', chain: [] });
  };
  const prepared = prepareRoutePreload();
  const event = pointerEvent('/next');

  pointerMove?.(event);
  pointerMove?.(event);
  const raw = await prepared.take('/next', '0a');

  assert.equal(fetches, 1);
  assert.ok(raw);
  assert.equal(
    await raw.response.text(),
    JSON.stringify({ path: '/next', chain: [] }),
  );
  assert.equal(await prepared.take('/next', '0a'), null);
  prepared.destroy();
});

test('hands off NDJSON after chunk one without waiting for EOF', async () => {
  let fetchSignal: AbortSignal | undefined;
  globalThis.fetch = async (_input, init) => {
    fetchSignal = init?.signal ?? undefined;
    return new Response(new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(new TextEncoder().encode(
          '{"path":"/next","chain":[]}\n',
        ));
      },
    }), {
      headers: { 'content-type': 'application/x-ndjson' },
      status: 200,
    });
  };
  const prepared = prepareRoutePreload();
  pointerMove?.(pointerEvent('/next'));

  const raw = await Promise.race([
    prepared.take('/next', '0a'),
    new Promise<null>((resolve) => setTimeout(() => resolve(null), 100)),
  ]);
  assert.ok(raw, 'chunk one should become adoptable before EOF');
  prepared.destroy();
  assert.equal(fetchSignal?.aborted, true);
});

test('aborts old path work when pointer intent changes', async () => {
  let aborted = false;
  globalThis.fetch = async (input, init) => {
    if (String(input).endsWith('/first')) {
      return await new Promise<Response>((resolve, reject) => {
        init?.signal?.addEventListener('abort', () => {
          aborted = true;
          reject(init.signal?.reason);
        }, { once: true });
      });
    }
    return jsonResponse({ path: '/second', chain: [] });
  };
  const prepared = prepareRoutePreload();

  pointerMove?.(pointerEvent('/first'));
  pointerMove?.(pointerEvent('/second'));
  const raw = await prepared.take('/second', '0a');

  assert.equal(aborted, true);
  assert.ok(raw);
  prepared.destroy();
});

test('rejects stale inventory and removes its listener on destroy', async () => {
  globalThis.fetch = async () =>
    jsonResponse({ path: '/next', chain: [] });
  const prepared = prepareRoutePreload();
  pointerMove?.(pointerEvent('/next'));

  assert.equal(await prepared.take('/next', '0b'), null);
  prepared.destroy();
  assert.equal(pointerMove, undefined);
});

test('intent rejection destroys the prepared listener', async () => {
  globalThis.fetch = async () =>
    jsonResponse({ path: '/next', chain: [] });
  prepareRoutePreload({
    onIntent: async () => {
      throw new Error('activation failed');
    },
  });
  pointerMove?.(pointerEvent('/next'));
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.equal(pointerMove, undefined);
});

test('aborts rejected response bodies before releasing the entry', async () => {
  let fetchSignal: AbortSignal | undefined;
  globalThis.fetch = async (_input, init) => {
    fetchSignal = init?.signal ?? undefined;
    return new Response('not a partial', {
      headers: { 'content-type': 'text/plain' },
      status: 200,
    });
  };
  const prepared = prepareRoutePreload();
  pointerMove?.(pointerEvent('/next'));

  assert.equal(await prepared.take('/next', '0a'), null);
  assert.equal(fetchSignal?.aborted, true);
  prepared.destroy();
});

test('idle expiration aborts and releases one prepared entry', async () => {
  let fetchSignal: AbortSignal | undefined;
  globalThis.fetch = async (_input, init) => {
    fetchSignal = init?.signal ?? undefined;
    return jsonResponse({ path: '/next', chain: [] });
  };
  const prepared = prepareRoutePreload();
  pointerMove?.(pointerEvent('/next'));
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.equal(prepared.has('/next'), true);

  const internals = prepared as unknown as {
    clearExpiry(): void;
    expirePending(): void;
  };
  internals.clearExpiry();
  internals.expirePending();
  assert.equal(prepared.has('/next'), false);
  assert.equal(fetchSignal?.aborted, true);
});

test('destroy aborts a request while take waits for handoff', async () => {
  let fetchSignal: AbortSignal | undefined;
  globalThis.fetch = async (_input, init) => {
    fetchSignal = init?.signal ?? undefined;
    return await new Promise<Response>((_resolve, reject) => {
      init?.signal?.addEventListener(
        'abort',
        () => reject(init.signal?.reason),
        { once: true },
      );
    });
  };
  const prepared = prepareRoutePreload();
  pointerMove?.(pointerEvent('/next'));
  const taking = prepared.take('/next', '0a');
  prepared.destroy();

  assert.equal(fetchSignal?.aborted, true);
  assert.equal(await taking, null);
});

function pointerEvent(pathname: string): PointerEvent {
  const anchor = {
    getAttribute: (name: string) => name === 'href' ? pathname : null,
    origin: location.origin,
    pathname,
    search: '',
    tagName: 'A',
  };
  return {
    composedPath: () => [anchor],
    pointerType: 'mouse',
  } as unknown as PointerEvent;
}

function jsonResponse(value: unknown): Response {
  return new Response(JSON.stringify(value), {
    headers: { 'content-type': 'application/json' },
    status: 200,
  });
}
