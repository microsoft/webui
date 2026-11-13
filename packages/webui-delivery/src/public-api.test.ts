// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import './browser-shim.js';
import { strict as assert } from 'node:assert';
import { readFileSync } from 'node:fs';
import { test, type TestContext } from 'node:test';
import type { RouterConfig, RouteResponsePolicy } from '@microsoft/webui-delivery/router.js';
import type { PartialTransport } from '@microsoft/webui-delivery/resources.js';

test('delivery has two task entrypoints and no compatibility facade', () => {
  const manifest = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8'));
  assert.equal(manifest.name, '@microsoft/webui-delivery');
  assert.deepEqual(Object.keys(manifest.exports).sort(), ['./resources.js', './router.js']);
  for (const entry of [
    '@microsoft/webui-delivery', '@microsoft/webui-delivery/core.js',
    '@microsoft/webui-delivery/preload.js', '@microsoft/webui-delivery/lazy-ndjson.js',
    '@microsoft/webui-delivery/lazy.js', '@microsoft/webui-framework/core.js',
  ]) {
    assert.throws(() => import.meta.resolve(entry), { code: 'ERR_PACKAGE_PATH_NOT_EXPORTED' });
  }
});

test('importing public delivery APIs does not construct or start a router', async t => {
  let listeners = 0;
  t.mock.method(window, 'addEventListener', () => { listeners++; });
  t.mock.method(document, 'addEventListener', () => { listeners++; });
  const api = await import('@microsoft/webui-delivery/router.js');
  assert.equal(typeof api.WebUIRouter, 'function');
  assert.equal(typeof api.parseQuery, 'function');
  assert.equal(Reflect.has(api, 'Router'), false);
  assert.equal(listeners, 0);
});

test('the public transport type configures routing', async () => {
  const { ndjson } = await import('@microsoft/webui-delivery/resources.js');
  const transport: PartialTransport = ndjson();
  const routing: RouterConfig = { transport };
  assert.equal(routing.transport, transport);
  for (const invalid of [0, -1, NaN, Infinity, 0.5]) {
    assert.throws(() => ndjson(invalid), RangeError);
  }
  const signal = new AbortController().signal;
  const body = await transport.read(Response.json({ state: { title: 'Ready' } }), signal);
  assert.deepEqual(body.data, { state: { title: 'Ready' } });
  assert.equal(body.records, undefined);
});

// Use only the exported router and its installed Navigation API handler.
async function navigationHarness(t: TestContext, config: RouterConfig) {
  const href = location.href;
  t.after(() => { location.href = href; });
  const { WebUIRouter } = await import('@microsoft/webui-delivery/router.js');
  let listener: ((event: NavigateEvent) => void) | undefined;
  t.mock.method(window.navigation, 'addEventListener', (
    type: string, callback: (event: NavigateEvent) => void,
  ) => { if (type === 'navigate') listener = callback; });
  window.__webui = {};
  const ready = new Promise<void>(resolve =>
    window.addEventListener('webui:route:navigated', () => resolve(), { once: true }),
  );
  const router = new WebUIRouter();
  t.after(() => router.destroy());
  router.start(config);
  await ready;
  const navigate = (path = '/next', signal = new AbortController().signal): Promise<void> => {
    let finished: Promise<void> | undefined;
    listener?.({
      canIntercept: true,
      hashChange: false,
      destination: { url: new URL(path, location.href).href },
      signal,
      intercept(options: { handler(): Promise<void> }) { finished = options.handler(); },
    } as NavigateEvent);
    assert.ok(finished, 'the public router should intercept navigation');
    return finished;
  };
  return Object.assign(navigate, { destroy: () => router.destroy() });
}

for (const status of [403, 200]) {
  test(`response policy sees ${status} before HTTP/HTML fallback`, async t => {
    const failure = new Error('manual recovery');
    const errors: unknown[] = [];
    let inspected = false;
    let requestSignal: AbortSignal | undefined;
    t.mock.method(globalThis, 'fetch', async (_input: unknown, init: RequestInit) => {
      requestSignal = init.signal ?? undefined;
      return new Response('Sign in manually', {
        status, headers: { 'content-type': 'text/html', 'x-recovery': 'manual' },
      });
    });
    const config: RouterConfig = {
      responsePolicy: {
        inspect(response, context) {
          inspected = true;
          assert.equal(response.status, status);
          assert.equal(response.headers.get('x-recovery'), 'manual');
          assert.equal(context.path, '/next');
          assert.equal(context.speculative, false);
          assert.equal(context.signal.aborted, false);
          throw failure;
        },
        onError(error: unknown) { errors.push(error); },
      },
    };
    const navigate = await navigationHarness(t, { timeoutMs: 1_000, ...config });
    const oldHref = location.href;
    await assert.rejects(navigate(), error => error === failure);
    assert.equal(inspected, true);
    assert.deepEqual(errors, [failure]);
    assert.equal(location.href, oldHref);
    assert.equal(requestSignal?.aborted, true, 'rejected bodies must be aborted');
  });
}

test('response policy reports network errors without treating old state as success', async t => {
  const failure = new TypeError('Network unavailable');
  const errors: unknown[] = [];
  t.mock.method(globalThis, 'fetch', async () => { throw failure; });
  const config: RouterConfig = {
    responsePolicy: {
      inspect() { assert.fail('network errors have no response'); },
      onError(error: unknown) { errors.push(error); },
    },
  };
  const navigate = await navigationHarness(t, { timeoutMs: 1_000, ...config });
  await assert.rejects(navigate(), error => error === failure);
  assert.deepEqual(errors, [failure]);
});

test('default HTTP failure behavior stays compatible without a response policy', async t => {
  t.mock.method(globalThis, 'fetch', async () => new Response('', { status: 403 }));
  t.mock.method(console, 'warn', () => {});
  const navigate = await navigationHarness(t, {});
  await navigate();
});

test('a configured policy cannot make an unhandled HTTP failure successful', async t => {
  const errors: unknown[] = [];
  t.mock.method(globalThis, 'fetch', async () => new Response('', { status: 403 }));
  const navigate = await navigationHarness(t, {
    responsePolicy: { inspect() {}, onError(error) { errors.push(error); } },
  });
  await assert.rejects(navigate(), /HTTP 403/);
  assert.equal(errors.length, 1);
});

for (const cancel of ['abort', 'supersede', 'timeout', 'destroy'] as const) {
  test(`async classification honors ${cancel} without stale recovery`, async t => {
    const errors: unknown[] = [];
    const started = Promise.withResolvers<void>();
    let responseSignal: AbortSignal | undefined;
    let cancelledBodies = 0;
    t.mock.method(globalThis, 'fetch', async () => new Response(
      new ReadableStream({ cancel() { cancelledBodies++; } }),
      { headers: { 'content-type': 'application/json' } },
    ));
    const controller = new AbortController();
    const navigate = await navigationHarness(t, {
      timeoutMs: cancel === 'timeout' ? 20 : 1_000,
      responsePolicy: {
        inspect(_response, context) {
          if (context.path === '/newer') throw new Error('newer failure');
          responseSignal = context.signal;
          started.resolve();
          return new Promise<void>(() => {});
        },
        onError(error) { errors.push(error); },
      },
    });
    const finished = navigate('/waiting', controller.signal);
    const rejected = assert.rejects(finished, {
      name: cancel === 'timeout' ? 'TimeoutError' : 'AbortError',
    });
    await started.promise;
    if (cancel === 'abort') controller.abort();
    if (cancel === 'destroy') navigate.destroy();
    if (cancel === 'supersede') await assert.rejects(navigate('/newer'), /newer failure/);
    await rejected;
    assert.equal(responseSignal?.aborted, true);
    assert.equal(cancelledBodies, cancel === 'supersede' ? 2 : 1);
    assert.equal(errors.length, cancel === 'abort' || cancel === 'destroy' ? 0 : 1);
    if (cancel === 'supersede') assert.match(String(errors[0]), /newer failure/);
  });
}

test('prepared responses use the same policy and are reclassified on adoption', async t => {
  const { routePreload, navigationCache } = await import('@microsoft/webui-delivery/router.js');
  let pointer: ((event: PointerEvent) => void) | undefined;
  t.mock.method(document, 'addEventListener', (type: string, handler: typeof pointer) => {
    if (type === 'pointermove') pointer = handler;
  });
  const inspected: boolean[] = [];
  const errors: unknown[] = [];
  const failure = new Error('manual recovery on navigation');
  const original = Response.json({ chain: [], templates: {} }, {
    headers: { 'x-classification': 'original' },
  });
  const policy: RouteResponsePolicy = {
    redirect: 'manual',
    inspect(response, context) {
      assert.equal(response, original, 'adoption must retain original response metadata');
      assert.equal(response.headers.get('x-classification'), 'original');
      inspected.push(context.speculative);
      if (!context.speculative) throw failure;
    },
    onError(error) { errors.push(error); },
  };
  const fetch = t.mock.method(globalThis, 'fetch', async (_input: unknown, init: RequestInit) => {
    assert.equal(init.redirect, 'manual');
    return original;
  });
  const preload = routePreload({ responsePolicy: policy });
  preload.prepare();
  pointer?.(pointerEvent('/next'));
  const navigate = await navigationHarness(t, {
    responsePolicy: policy, preload, cache: navigationCache(),
  });
  await assert.rejects(navigate(), error => error === failure);
  assert.deepEqual(inspected, [true, false]);
  assert.deepEqual(errors, [failure]);
  assert.equal(fetch.mock.callCount(), 1);
});

test('unclassified prepared work is discarded before a manual-redirect navigation', async t => {
  const { routePreload, navigationCache } = await import('@microsoft/webui-delivery/router.js');
  let pointer: ((event: PointerEvent) => void) | undefined;
  t.mock.method(document, 'addEventListener', (type: string, handler: typeof pointer) => {
    if (type === 'pointermove') pointer = handler;
  });
  const redirects: (RequestRedirect | undefined)[] = [];
  t.mock.method(globalThis, 'fetch', async (_input: unknown, init: RequestInit) => {
    redirects.push(init.redirect);
    return Response.json({ chain: [] });
  });
  const preload = routePreload();
  preload.prepare();
  pointer?.(pointerEvent('/next'));
  const navigate = await navigationHarness(t, {
    preload, cache: navigationCache(),
    responsePolicy: {
      redirect: 'manual',
      inspect() { throw new Error('classified fresh request'); },
      onError() {},
    },
  });
  await assert.rejects(navigate(), /classified fresh request/);
  assert.deepEqual(redirects, [undefined, 'manual']);
});

test('speculative errors do not present recovery and are retried on actual navigation', async t => {
  const { routePreload, navigationCache } = await import('@microsoft/webui-delivery/router.js');
  let pointer: ((event: PointerEvent) => void) | undefined;
  t.mock.method(document, 'addEventListener', (type: string, handler: typeof pointer) => {
    if (type === 'pointermove') pointer = handler;
  });
  const speculativeDone = Promise.withResolvers<void>();
  const inspected: boolean[] = [];
  const errors: unknown[] = [];
  t.mock.method(globalThis, 'fetch', async (_input: unknown, init: RequestInit) => {
    init.signal?.addEventListener('abort', () => speculativeDone.resolve(), { once: true });
    return new Response('denied', { status: 403 });
  });
  const navigate = await navigationHarness(t, {
    preload: routePreload(), cache: navigationCache(),
    responsePolicy: {
      inspect(_response, context) {
        inspected.push(context.speculative);
        throw new Error('denied');
      },
      onError(error) { errors.push(error); },
    },
  });
  pointer?.(pointerEvent('/next'));
  await speculativeDone.promise;
  assert.equal(errors.length, 0);
  await assert.rejects(navigate(), /denied/);
  assert.deepEqual(inspected, [true, false]);
  assert.equal(errors.length, 1);
});

test('prepared work from another admission function is not adopted', async t => {
  const { routePreload, navigationCache } = await import('@microsoft/webui-delivery/router.js');
  let pointer: ((event: PointerEvent) => void) | undefined;
  t.mock.method(document, 'addEventListener', (type: string, handler: typeof pointer) => {
    if (type === 'pointermove') pointer = handler;
  });
  const policy: RouteResponsePolicy = {
    inspect(_response, context) {
      if (!context.speculative) throw new Error('fresh admitted request');
    },
    onError() {},
  };
  const fetch = t.mock.method(globalThis, 'fetch', async () => Response.json({ chain: [] }));
  const preload = routePreload({ responsePolicy: policy, shouldIntercept: () => true });
  preload.prepare();
  pointer?.(pointerEvent('/next'));
  const navigate = await navigationHarness(t, {
    responsePolicy: policy, shouldIntercept: () => true,
    preload, cache: navigationCache(),
  });
  await assert.rejects(navigate(), /fresh admitted request/);
  assert.equal(fetch.mock.callCount(), 2);
});

function pointerEvent(pathname: string): PointerEvent {
  return {
    composedPath: () => [{
      tagName: 'A', origin: location.origin, pathname, search: '',
      href: new URL(pathname, location.href).href,
      getAttribute: (name: string) => name === 'href' ? pathname : null,
    }],
    pointerType: 'mouse',
  } as unknown as PointerEvent;
}
