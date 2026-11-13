// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import './browser-shim.js';

import { strict as assert } from 'node:assert';
import { test } from 'node:test';

import { WebUIRouter } from './router.js';
import {
  cancelDeferredStream,
  readStreamingPartial as readPartial,
  startDeferredStream,
  type StreamingContext,
} from './streaming.js';
import { ndjson } from './ndjson.js';
import {
  fetchComponentTemplates,
  injectCssLinks,
  registerTemplatesAndStyles,
  waitForTemplateReadiness,
} from './templates.js';
import { navigationCache, type PartialResponse } from './cache.js';

test('injected stylesheet links resolve against the runtime asset base', t => {
  const appended: { rel?: string; href?: string }[] = [];
  t.mock.method(document.head, 'appendChild', (link: { rel?: string; href?: string }) => {
    appended.push(link);
    return link;
  });
  const runtime = window.__webui ??= {};
  const previousBase = runtime.assetBase;
  const documentBase = Object.getOwnPropertyDescriptor(document, 'baseURI');
  Object.defineProperty(document, 'baseURI', { configurable: true, value: 'https://site.example/news/' });
  runtime.assetBase = 'https://cdn.example/assets/';
  try {
    const injected = new Set<string>();
    injectCssLinks({ css: ['app-shell.css', 'app-shell.css', 'https://fonts.example/a.css'] }, injected);
    assert.deepEqual(appended.map(link => [link.rel, link.href]), [
      ['stylesheet', 'https://cdn.example/assets/app-shell.css'],
      ['stylesheet', 'https://fonts.example/a.css'],
    ]);
    assert.deepEqual([...injected], ['app-shell.css', 'https://fonts.example/a.css'], 'raw hrefs stay the keys');
    runtime.assetBase = '/static/';
    injectCssLinks({ css: ['page.css'] }, injected);
    assert.equal(appended.at(-1)?.href, 'https://site.example/static/page.css');
    delete runtime.assetBase;
    injectCssLinks({ css: ['local.css'] }, injected);
    assert.equal(appended.at(-1)?.href, 'https://site.example/news/local.css');
  } finally {
    if (documentBase) Object.defineProperty(document, 'baseURI', documentBase);
    else Reflect.deleteProperty(document, 'baseURI');
    if (previousBase === undefined) delete runtime.assetBase;
    else runtime.assetBase = previousBase;
  }
});

test('resource-only styles use the response metadata base and reject an invalid configured base', t => {
  const appended: { href?: string }[] = [];
  t.mock.method(document.head, 'appendChild', (link: { href?: string }) => appended.push(link));
  t.mock.method(document, 'querySelector', () => ({ content: '/override/' }));
  const runtime = window.__webui ??= {};
  const previous = runtime.assetBase;
  delete runtime.assetBase;
  try {
    const injected = new Set<string>();
    injectCssLinks({ css: ['card.css'] }, injected);
    assert.equal(appended[0].href, 'http://localhost/override/card.css');
    runtime.assetBase = 'https://:443/';
    assert.throws(() => injectCssLinks({ css: ['missing.css'] }, injected), TypeError);
    assert.equal(appended.length, 1, 'an invalid base cannot fall back to another deployment');
    runtime.assetBase = '/correct/';
    injectCssLinks({ css: ['missing.css'] }, injected);
    assert.equal(appended[1].href, 'http://localhost/correct/missing.css', 'a failed URL does not poison the retry');
  } finally {
    if (previous === undefined) delete runtime.assetBase;
    else runtime.assetBase = previous;
  }
});

test('a failed template request cancels its unread response', async t => {
  let signal: AbortSignal | null | undefined;
  t.mock.method(globalThis, 'fetch', async (_url: unknown, init?: RequestInit) => {
    signal = init?.signal;
    return new Response('Unavailable', { status: 503 });
  });
  await assert.rejects(
    fetchComponentTemplates(['example-card'], '', '/templates', '', () => {}),
    /Template request failed: 503/,
  );
  assert.equal(signal?.aborted, true);
});

interface RegistrationDetail {
  templates: Record<string, unknown>;
  waitUntil(promise: PromiseLike<unknown>): void;
}

function readStreamingPartial(
  response: Response,
  path: string,
  context: StreamingContext,
  signal?: AbortSignal,
) {
  response.headers.set('content-type', 'application/x-ndjson');
  return readPartial(response, path, context, signal, ndjson());
}

test('deferred state releases an over-budget cache entry before the stream closes', async () => {
  const cache = navigationCache({ staleTime: 60_000, maxBytes: 2_000 });
  let stream!: ReadableStreamDefaultController<Uint8Array>;
  let deferred: Promise<void> | null = null;
  const accounted = Promise.withResolvers<void>();
  const context: StreamingContext = {
    navGeneration: 1, currentRequestPath: '/budget', activeChain: [],
    nonce: '', injectedCss: new Set(),
    setDeferredReader(reader) { if (reader) deferred = reader; },
    setDeferredGeneration() {}, updateInventory() {},
    refreshCacheStream(path, data, states) {
      cache.refreshStream(path, data, states);
      accounted.resolve();
    },
    settleCacheStream(path, data, completed, accounted) { cache.settleStream(path, data, completed, accounted); },
  };
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      stream = controller;
      controller.enqueue(new TextEncoder().encode(
        '{"path":"/budget","chain":[{"component":"test-budget","path":"/budget","params":{}}],"templates":{}}\n',
      ));
    },
  });
  const data = await readStreamingPartial(new Response(body), '/budget', context);
  assert.ok(data);
  cache.store('/budget', data, true, true);
  assert.equal(cache.has('/budget'), true);
  startDeferredStream(data);
  stream.enqueue(new TextEncoder().encode(JSON.stringify({ states: [{ text: 'x'.repeat(2_000) }] }) + '\n'));
  await accounted.promise;
  assert.equal(cache.has('/budget'), false);
  assert.equal(data.chain?.[0].state?.text, 'x'.repeat(2_000), 'eviction must not discard navigation state');
  stream.close();
  await deferred;
});

test('a stream context without incremental accounting measures grown state at settlement', async () => {
  const cache = navigationCache({ staleTime: 60_000, maxBytes: 2_000 });
  let stream!: ReadableStreamDefaultController<Uint8Array>;
  let deferred: Promise<void> | null = null;
  let accountingClaim: boolean | undefined;
  const context: StreamingContext = {
    navGeneration: 1, currentRequestPath: '/fallback', activeChain: [],
    nonce: '', injectedCss: new Set(),
    setDeferredReader(reader) { if (reader) deferred = reader; },
    setDeferredGeneration() {}, updateInventory() {},
    settleCacheStream(path, data, completed, accounted) {
      accountingClaim = accounted;
      cache.settleStream(path, data, completed, accounted);
    },
  };
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      stream = controller;
      controller.enqueue(new TextEncoder().encode(
        '{"path":"/fallback","chain":[{"component":"test-budget","path":"/fallback","params":{}}],"templates":{}}\n',
      ));
    },
  });
  const data = await readStreamingPartial(new Response(body), '/fallback', context);
  assert.ok(data);
  cache.store('/fallback', data, true, true);
  startDeferredStream(data);
  stream.enqueue(new TextEncoder().encode(JSON.stringify({ states: [{ text: 'x'.repeat(2_000) }] }) + '\n'));
  stream.close();
  await deferred;
  assert.equal(accountingClaim, false);
  assert.equal(cache.has('/fallback'), false);
  assert.equal(data.chain?.[0].state?.text, 'x'.repeat(2_000));
});

test('template registration waits for synchronous runtime readiness', async () => {
  let release!: () => void;
  const blocker = new Promise<void>(resolve => {
    release = resolve;
  });
  let waitUntil: RegistrationDetail['waitUntil'] | undefined;
  const listener = (event: Event): void => {
    const detail = (event as CustomEvent<RegistrationDetail>).detail;
    waitUntil = detail.waitUntil;
    detail.waitUntil(blocker);
  };
  window.addEventListener('webui:templates-registered', listener);
  try {
    const ready = registerTemplatesAndStyles(
      { templates: { 'delayed-card': { h: '<p>Delayed</p>' } } },
      '',
      () => {},
    );
    assert.ok(ready);
    let settled = false;
    void ready.then(() => {
      settled = true;
    });
    await Promise.resolve();
    assert.equal(settled, false);

    release();
    await ready;
    assert.equal(settled, true);
    assert.throws(
      () => waitUntil?.(Promise.resolve()),
      /must be called during event dispatch/,
    );
  } finally {
    window.removeEventListener('webui:templates-registered', listener);
  }
});

test('inventory is acknowledged only after resource readiness succeeds', async () => {
  const preparation = Promise.withResolvers<void>();
  const listener = (event: Event): void => {
    (event as CustomEvent<RegistrationDetail>).detail.waitUntil(preparation.promise);
  };
  window.addEventListener('webui:templates-registered', listener);
  const inventories: string[] = [];
  try {
    const ready = registerTemplatesAndStyles(
      { templates: { 'inventory-card': { h: '<p></p>' } }, inventory: '04' },
      '',
      inventory => inventories.push(inventory),
    );
    assert.ok(ready);
    assert.deepEqual(inventories, []);
    preparation.resolve();
    await ready;
    assert.deepEqual(inventories, ['04']);
  } finally {
    window.removeEventListener('webui:templates-registered', listener);
  }
});

test('failed readiness cannot acknowledge templates that did not become usable', async () => {
  const failure = new Error('definition failed');
  const listener = (event: Event): void => {
    (event as CustomEvent<RegistrationDetail>).detail.waitUntil(Promise.reject(failure));
  };
  window.addEventListener('webui:templates-registered', listener);
  let acknowledged = false;
  try {
    const ready = registerTemplatesAndStyles(
      { templates: { 'rejected-inventory-card': { h: '<p></p>' } }, inventory: '08' },
      '',
      () => { acknowledged = true; },
    );
    assert.ok(ready);
    await assert.rejects(ready, failure);
    assert.equal(acknowledged, false);
  } finally {
    window.removeEventListener('webui:templates-registered', listener);
  }
});

test('resource publication and cached responses reuse renderer-canonical templates', () => {
  const original = window.__webuiPrepareTemplates;
  const canonical = { h: '<p>Canonical</p>' };
  window.__webuiPrepareTemplates = templates => { templates['canonical-card'] = canonical; };
  const data = { templates: { 'canonical-card': { h: '<p>Canonical</p>' } } };
  try {
    registerTemplatesAndStyles(data, '', () => {});
    assert.equal(data.templates['canonical-card'], canonical);
  } finally {
    window.__webuiPrepareTemplates = original;
  }
});

test('renderer preparation failures do not publish invalid templates or inventory', () => {
  const original = window.__webuiPrepareTemplates;
  const failure = new Error('conflicting resource');
  window.__webuiPrepareTemplates = () => { throw failure; };
  let acknowledged = false;
  try {
    assert.throws(() => registerTemplatesAndStyles(
      { templates: { 'unpublished-card': { h: '<p></p>' } }, inventory: '20' },
      '',
      () => { acknowledged = true; },
    ), failure);
    assert.equal(acknowledged, false);
    assert.equal(Reflect.get(window.__webui ?? {}, 'templates')?.['unpublished-card'], undefined);
  } finally {
    window.__webuiPrepareTemplates = original;
  }
});

for (const tail of ['', '{"states":', '{"error":"data unavailable"}\n']) {
  test(`an incomplete or failed state tail cannot make a route cache entry complete: ${JSON.stringify(tail)}`, async () => {
    const warnings: unknown[] = [];
    const original = console.warn;
    console.warn = (...args: unknown[]) => { warnings.push(args); };
    let completed = false;
    let pending: Promise<void> | null = null;
    const response = new Response(
      `{"chain":[],"templates":{}}\n${tail}`,
    );
    const context: StreamingContext = {
      navGeneration: 1,
      currentRequestPath: '/failed-tail',
      activeChain: [],
      nonce: '',
      injectedCss: new Set(),
      setDeferredReader(value) { if (value) pending = value; },
      setDeferredGeneration() {},
      updateInventory() {},
      settleCacheStream(_path, _data, settled) { completed = settled; },
    };
    try {
      if (tail.endsWith('\n')) {
        await assert.rejects(readStreamingPartial(response, '/failed-tail', context), /could not complete/);
      } else {
        const data = await readStreamingPartial(response, '/failed-tail', context);
        assert.ok(data);
        startDeferredStream(data);
        assert.ok(pending);
        await pending;
        assert.equal(warnings.length, 1);
      }
      assert.equal(completed, false);
      assert.equal(response.body?.locked, false);
    } finally {
      console.warn = original;
    }
  });
}

test('streaming registration rejects a stale navigation after readiness wait', async () => {
  let release!: () => void;
  let registrationSeen!: () => void;
  const blocker = new Promise<void>(resolve => {
    release = resolve;
  });

  const seen = new Promise<void>(resolve => {
    registrationSeen = resolve;
  });
  const listener = (event: Event): void => {
    const detail = (event as CustomEvent<RegistrationDetail>).detail;
    detail.waitUntil(blocker);
    registrationSeen();
  };
  window.addEventListener('webui:templates-registered', listener);

  let generation = 1;
  let deferredReaderSet = false;
  const context: StreamingContext = {
    get navGeneration() {
      return generation;
    },
    currentRequestPath: '/old',
    activeChain: [],
    nonce: '',
    injectedCss: new Set(),
    setDeferredReader(reader) {
      deferredReaderSet = reader !== null;
    },
    setDeferredGeneration() {},
    updateInventory() {},
    settleCacheStream() {},
  };
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(new TextEncoder().encode(
        `${JSON.stringify({
          chain: [],
          css: ['/stale.css'],
          templates: { 'stale-card': { h: '<p>Stale</p>' } },
        })}\n`,
      ));
      controller.close();
    },
  });

  try {
    const pending = readStreamingPartial(
      new Response(body),
      '/old',
      context,
    );
    await seen;
    assert.equal(context.injectedCss.has('/stale.css'), true);
    generation = 2;
    release();

    assert.equal(await pending, null);
    assert.equal(deferredReaderSet, false);
  } finally {
    window.removeEventListener('webui:templates-registered', listener);
  }
});

test('streaming defers chunk-two state until route commit', async () => {
  let release!: () => void;
  let registrationSeen!: () => void;
  const blocker = new Promise<void>(resolve => {
    release = resolve;
  });
  const seen = new Promise<void>(resolve => {
    registrationSeen = resolve;
  });
  const listener = (event: Event): void => {
    const detail = (event as CustomEvent<RegistrationDetail>).detail;
    detail.waitUntil(blocker);
    registrationSeen();
  };
  window.addEventListener('webui:templates-registered', listener);

  class StatefulCard extends HTMLElement {}
  const tag = 'stream-readiness-card';
  if (!customElements.get(tag)) customElements.define(tag, StatefulCard);
  const oldReceived: Record<string, unknown>[] = [];
  const newReceived: Record<string, unknown>[] = [];
  const oldCard = document.createElement(tag) as HTMLElement & {
    setState(state: Record<string, unknown>): void;
  };
  const newCard = document.createElement(tag) as HTMLElement & {
    setState(state: Record<string, unknown>): void;
  };
  oldCard.setState = state => oldReceived.push(state);
  newCard.setState = state => newReceived.push(state);
  const activeChain = [{
    component: tag,
    path: '/old',
    params: {},
    el: document.createElement('div'),
    compEl: oldCard,
  }];
  let deferredReader: Promise<void> | null = null;
  const context: StreamingContext = {
    navGeneration: 1,
    currentRequestPath: '/new',
    activeChain,
    nonce: '',
    injectedCss: new Set(),
    setDeferredReader(reader) {
      deferredReader = reader;
    },
    setDeferredGeneration() {},
    updateInventory() {},
    settleCacheStream() {},
  };
  let stream!: ReadableStreamDefaultController<Uint8Array>;
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      stream = controller;
      controller.enqueue(new TextEncoder().encode(
        `${JSON.stringify({
          chain: [{ component: tag, path: '/new' }],
          templates: { [tag]: { h: '<p>New</p>' } },
        })}\n`,
      ));
    },
  });

  try {
    const pending = readStreamingPartial(
      new Response(body),
      '/new',
      context,
    );
    await seen;
    stream.enqueue(new TextEncoder().encode(
      `${JSON.stringify({ states: [{ message: 'new state' }] })}\n`,
    ));
    stream.close();
    release();
    const data = await pending;
    assert.ok(data);
    assert.deepEqual(oldReceived, []);

    activeChain.splice(0, 1, {
      component: tag,
      path: '/new',
      params: {},
      el: document.createElement('div'),
      compEl: newCard,
    });
    startDeferredStream(data);
    assert.equal(data._deferredStream, undefined);
    assert.ok(deferredReader);
    await deferredReader;
    assert.deepEqual(oldReceived, []);
    assert.deepEqual(newReceived, [{ message: 'new state' }]);
    assert.deepEqual(data.chain?.[0].state, { message: 'new state' });
  } finally {
    window.removeEventListener('webui:templates-registered', listener);
  }
});

test('same-read chunk-two state is folded into commit data once', async () => {
  const received: Record<string, unknown>[] = [];
  const component = document.createElement('div') as unknown as HTMLElement & {
    setState(state: Record<string, unknown>): void;
  };
  component.setState = state => received.push(state);
  let deferredReader: Promise<void> | null = null;
  const context: StreamingContext = {
    navGeneration: 1,
    currentRequestPath: '/same-read',
    activeChain: [{
      component: 'same-read-card',
      path: '/same-read',
      params: {},
      el: document.createElement('div'),
      compEl: component,
    }],
    nonce: '',
    injectedCss: new Set(),
    setDeferredReader(reader) {
      deferredReader = reader;
    },
    setDeferredGeneration() {},
    updateInventory() {},
    settleCacheStream() {},
  };
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(new TextEncoder().encode(
        `${JSON.stringify({
          chain: [{
            component: 'same-read-card',
            path: '/same-read',
            params: {},
          }],
          path: '/same-read',
          templates: {},
        })}\n${JSON.stringify({
          states: [{ message: 'folded' }],
        })}\n`,
      ));
      controller.close();
    },
  });

  const data = await readStreamingPartial(
    new Response(body),
    '/same-read',
    context,
  );
  assert.ok(data);
  assert.deepEqual(data.chain?.[0].state, { message: 'folded' });
  assert.deepEqual(received, []);
  startDeferredStream(data);
  assert.ok(deferredReader);
  await deferredReader;
  assert.deepEqual(received, []);
});

for (const aborted of [false, true]) for (const started of [false, true]) {
  test(`stream settlement waits for ${aborted ? 'aborted' : 'stale'} ${started ? 'active' : 'unread'} cancellation`, async () => {
    const cancelling = Promise.withResolvers<void>();
    const release = Promise.withResolvers<void>();
    const settlements: Array<{ path: string; data: PartialResponse; completed: boolean }> = [];
    let generation = 1;
    const controller = new AbortController();
    let deferred: Promise<void> | null = null;
    const context: StreamingContext = {
      get navGeneration() { return generation; },
      currentRequestPath: '/cancelled',
      activeChain: [],
      nonce: '',
      injectedCss: new Set(),
      setDeferredReader(reader) { deferred = reader; },
      setDeferredGeneration() {},
      updateInventory() {},
      settleCacheStream(path, data, completed) {
        settlements.push({ path, data, completed });
      },
    };
    const body = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(new TextEncoder().encode(
          '{"path":"/cancelled","chain":[],"templates":{}}\n',
        ));
      },
      cancel() {
        cancelling.resolve();
        return release.promise;
      },
    });
    const data = await readStreamingPartial(new Response(body), '/cancelled', context, controller.signal);
    assert.ok(data);
    if (aborted) controller.abort();
    else generation++;
    let pending: Promise<void>;
    if (started) {
      startDeferredStream(data);
      assert.ok(deferred);
      pending = deferred;
    } else {
      pending = cancelDeferredStream(data);
    }
    await cancelling.promise;
    assert.deepEqual(settlements, [], 'pending cancellation must retain live ownership');
    assert.equal(body.locked, false, 'the transport releases its reader before asynchronous cancellation settles');
    release.resolve();
    await pending;
    assert.equal(body.locked, false);
    assert.deepEqual(settlements, [{ path: '/cancelled', data, completed: false }]);
  });
}

test('template readiness releases an aborted navigation without cancelling shared work', async () => {
  const controller = new AbortController();
  const sharedWork = new Promise<void>(() => {});
  const pending = waitForTemplateReadiness(sharedWork, controller.signal);
  controller.abort();
  assert.equal(await pending, false);
});

test('streaming registration releases its reader when readiness rejects', async () => {
  const rejection = new Error('runtime preparation failed');
  const listener = (event: Event): void => {
    const detail = (event as CustomEvent<RegistrationDetail>).detail;
    detail.waitUntil(Promise.reject(rejection));
  };
  window.addEventListener('webui:templates-registered', listener);

  let cancelled = false;
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(new TextEncoder().encode(
        `${JSON.stringify({
          chain: [],
          templates: { 'failed-card': { h: '<p>Failed</p>' } },
        })}\n`,
      ));
    },
    cancel() {
      cancelled = true;
    },
  });
  const response = new Response(body);
  const context: StreamingContext = {
    navGeneration: 1,
    currentRequestPath: '/failed',
    activeChain: [],
    nonce: '',
    injectedCss: new Set(),
    setDeferredReader() {},
    setDeferredGeneration() {},
    updateInventory() {},
    settleCacheStream() {},
  };

  try {
    await assert.rejects(
      readStreamingPartial(response, '/failed', context),
      rejection,
    );
    assert.equal(cancelled, true);
    assert.equal(response.body?.locked, false);
  } finally {
    window.removeEventListener('webui:templates-registered', listener);
  }
});

test('router cancels an unread deferred stream when commit rejects', async () => {
  let cancelled = false;
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(new TextEncoder().encode(
        `${JSON.stringify({
          chain: [{ component: 'failed-card', path: '/failed' }],
          path: '/failed',
          templates: {},
        })}\n`,
      ));
    },
    cancel() {
      cancelled = true;
    },
  });
  const response = new Response(body);
  const context: StreamingContext = {
    navGeneration: 1,
    currentRequestPath: '/failed',
    activeChain: [],
    nonce: '',
    injectedCss: new Set(),
    setDeferredReader() {},
    setDeferredGeneration() {},
    updateInventory() {},
    settleCacheStream() {},
  };
  const data = await readStreamingPartial(response, '/failed', context);
  assert.ok(data);
  assert.equal(data._deferredStream, true);

  const failure = new Error('loader failed');
  const router = new WebUIRouter() as unknown as {
    clearSsrPreloads(): void;
    commitWithData(): Promise<boolean>;
    fetchPartial(): Promise<typeof data>;
    handleNavigation(target: { requestPath: string }): Promise<void>;
    isInitialNavigation: boolean;
  };
  router.isInitialNavigation = false;
  router.clearSsrPreloads = () => {};
  (document.body as unknown as {
    querySelectorAll(): unknown[];
  }).querySelectorAll = () => [];
  router.fetchPartial = async () => data;
  router.commitWithData = async () => {
    throw failure;
  };

  await assert.rejects(
    router.handleNavigation({ requestPath: '/failed' }),
    failure,
  );
  assert.equal(cancelled, true);
  assert.equal(data._deferredStream, undefined);
  assert.equal(response.body?.locked, false);
});

test('template registration remains immediate without a runtime listener', () => {
  assert.equal(
    registerTemplatesAndStyles(
      { templates: { 'immediate-card': { h: '<p>Immediate</p>' } } },
      '',
      () => {},
    ),
    undefined,
  );
});
