// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import './browser-shim.js';
import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { WebUIRouter } from './core.js';
import { ndjson } from './streaming.js';
import { startDeferredStream } from './partial-response.js';
import { createNavigationCache } from './cache.js';
import { readJsonPartial } from './partial-json.js';
import { routeBoundaries } from './boundaries.js';
import { viewTransitions } from './view-transitions.js';
import { validateRoutes } from './diagnostics.js';
import { PendingState } from './pending.js';

interface PartialRouter {
  fetchPartial(path: string): Promise<unknown>;
  deferredReader: Promise<void> | null;
  partialControllers: Set<AbortController>;
}

test('lean routing requests JSON without opting into streaming negotiation', async t => {
  window.__webui = Object.assign({ inventory: '', nonce: '' }, { templates: {} });
  let accept = '';
  t.mock.method(globalThis, 'fetch', async (_input: unknown, options?: RequestInit) => {
    accept = new Headers(options?.headers).get('accept') ?? '';
    return Response.json({ chain: [], templates: {}, path: '/json' });
  });
  const router = new WebUIRouter();
  const data = await (router as unknown as PartialRouter).fetchPartial('/json');
  assert.ok(data);
  assert.equal(accept, 'application/json');
  router.destroy();
});

test('selecting NDJSON readies the partial decoder without changing commit ownership', async t => {
  window.__webui = Object.assign({ inventory: '', nonce: '' }, { templates: {} });
  let accept = '';
  t.mock.method(globalThis, 'fetch', async (_input: unknown, options?: RequestInit) => {
    accept = new Headers(options?.headers).get('accept') ?? '';
    return new Response('{"chain":[],"templates":{},"path":"/stream"}\n{"states":[]}\n', {
      headers: { 'content-type': 'application/x-ndjson' },
    });
  });
  const router = new WebUIRouter({ streaming: ndjson });
  const internals = router as unknown as PartialRouter;
  const data = await internals.fetchPartial('/stream') as Parameters<typeof startDeferredStream>[0];
  assert.ok(data);
  assert.equal(accept, 'application/x-ndjson, application/json');
  assert.equal(internals.deferredReader, null);
  startDeferredStream(data);
  assert.ok(internals.deferredReader);
  await internals.deferredReader;
  router.destroy();
});

test('unselected policy flags fail with actionable capability errors', () => {
  for (const [config, capability] of [
    [{ cache: {} }, 'cache'],
    [{ preload: true }, 'cache'],
    [{ actions: true }, 'actions'],
    [{ dev: true }, 'diagnostics'],
  ] as const) {
    assert.throws(() => new WebUIRouter().start(config), new RegExp(`provide the ${capability} capability`));
  }
});

test('explicit cache and boundary implementations can be composed independently', () => {
  const cache = createNavigationCache({ staleTime: 1000, gcTime: 2000, maxEntries: 2 });
  assert.equal(cache.has('/missing'), false);
  const previous = Object.getOwnPropertyDescriptor(document.body, 'querySelectorAll');
  Object.defineProperty(document.body, 'querySelectorAll', { configurable: true, value: () => [] });
  try {
    assert.equal(routeBoundaries.pending([], '/missing'), null);
    assert.equal(routeBoundaries.error([], '/missing'), null);
  } finally {
    if (previous) Object.defineProperty(document.body, 'querySelectorAll', previous);
    else Reflect.deleteProperty(document.body, 'querySelectorAll');
  }
  const pending = routeBoundaries.create();
  assert.ok(!(pending instanceof Promise));
  pending.destroy();
  validateRoutes([]);
});

for (const start of [false, true]) {
  test(`the partial deadline releases a ${start ? 'started' : 'unread'} state tail`, async t => {
    t.mock.timers.enable({ apis: ['setTimeout'] });
    window.__webui = Object.assign({ inventory: '', nonce: '' }, { templates: {} });
    let signal: AbortSignal | null | undefined;
    let cancelled = false;
    const response = new Response(new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(new TextEncoder().encode('{"chain":[],"templates":{},"path":"/slow"}\n'));
      },
      cancel() { cancelled = true; },
    }), { headers: { 'content-type': 'application/x-ndjson' } });
    t.mock.method(globalThis, 'fetch', async (_input: unknown, options?: RequestInit) => {
      signal = options?.signal;
      return response;
    });
    const router = new WebUIRouter({ streaming: ndjson });
    const internals = router as unknown as PartialRouter;
    try {
      const data = await internals.fetchPartial('/slow') as Parameters<typeof startDeferredStream>[0];
      assert.ok(data);
      if (start) startDeferredStream(data);
      const pending = internals.deferredReader;
      if (start) assert.ok(pending);
      else assert.equal(pending, null);
      t.mock.timers.tick(10_001);
      await pending;
      assert.equal(signal?.aborted, true);
      assert.equal(cancelled, true);
      assert.equal(response.body?.locked, false);
      assert.equal(internals.partialControllers.size, 0);
    } finally {
      router.destroy();
    }
  });
}

test('concurrent boundary requests share one preparation and destruction cannot retain its late result', async () => {
  const preparation = Promise.withResolvers<PendingState>();
  let creations = 0;
  let destroyed = false;
  const pending = new PendingState();
  pending.destroy = () => { destroyed = true; };
  const router = new WebUIRouter({
    boundaries: {
      ...routeBoundaries,
      create: () => { creations++; return preparation.promise; },
    },
  });
  const internals = router as unknown as {
    pendingState(): Promise<PendingState>;
    pending: PendingState | null;
  };
  const first = internals.pendingState();
  const second = internals.pendingState();
  await Promise.resolve();
  assert.equal(creations, 1);
  router.destroy();
  preparation.resolve(pending);
  await assert.rejects(first, { name: 'AbortError' });
  await assert.rejects(second, { name: 'AbortError' });
  assert.equal(destroyed, true);
  assert.equal(internals.pending, null);
});

test('the transition capability guards unsupported browsers and removes its own style', t => {
  const original = document.startViewTransition;
  Object.defineProperty(document, 'startViewTransition', { configurable: true, writable: true, value: undefined });
  assert.equal(viewTransitions.supported(), false);
  viewTransitions.install()();
  let removed = false;
  const style = { textContent: '', nonce: '', remove() { removed = true; } };
  t.mock.method(document, 'createElement', () => style as unknown as HTMLStyleElement);
  t.mock.method(document.head, 'appendChild', <T extends Node>(node: T): T => node);
  Object.defineProperty(document, 'startViewTransition', {
    configurable: true, writable: true,
    value: (commit: () => void) => { commit(); return { updateCallbackDone: Promise.resolve() }; },
  });
  try {
    window.__webui = { nonce: 'nonce' };
    const dispose = viewTransitions.install();
    assert.equal(style.textContent, '@view-transition { navigation: none; }');
    assert.equal(style.nonce, 'nonce');
    let committed = false;
    viewTransitions.startViewTransition(() => { committed = true; });
    assert.equal(committed, true);
    dispose();
    assert.equal(removed, true);
  } finally {
    document.startViewTransition = original;
  }
});

test('the JSON resource reader validates shape and respects cancellation', async () => {
  const context = { nonce: '', injectedCss: new Set<string>(), updateInventory() {} };
  await assert.rejects(readJsonPartial(Response.json([]), context), /must be a JSON object/);
  const controller = new AbortController();
  controller.abort();
  assert.equal(
    await readJsonPartial(Response.json({ templates: { 'x-cancelled': { h: '' } } }), context, controller.signal),
    null,
  );
});
