// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import './browser-shim.js';
import { strict as assert } from 'node:assert';
import { beforeEach, test } from 'node:test';
import { createLazyLoader } from './lazy-loader.js';
import { lazyFragment, type LazyRecord } from './lazy-data.js';
import { lazyNdjson } from './lazy-ndjson.js';
import { clearInventory } from './inventory.js';

const owner = 'app-shell';
const id = 'settings';
const fragment = lazyFragment(owner, id);

function record(state?: Record<string, unknown>): LazyRecord {
  return {
    type: 'webui-lazy', version: 1, owner, fragment, buildId: 'build',
    inventory: '01', styleInventory: '', required: [fragment],
    templates: { [fragment]: { h: '<p>Lazy</p>', f: fragment, o: owner, v: 3 } },
    ...(state === undefined ? {} : { state }),
  };
}

function templates(): Record<string, unknown> {
  return Reflect.get(window.__webui ?? {}, 'templates') as Record<string, unknown>;
}

beforeEach(() => {
  window.__webui = Object.assign({ buildId: 'build', inventory: '', styleInventory: '', state: {} }, { templates: {} });
  window.__webuiResolveResources = undefined;
  window.__webuiPrepareResources = undefined;
  window.__webuiPrepareTemplates = undefined;
  window.__webuiRetainTemplate = undefined;
});

test('the former deferred exports and response discriminator are not aliases', async t => {
  assert.throws(() => import.meta.resolve('@microsoft/webui-router/deferred.js'), {
    code: 'ERR_PACKAGE_PATH_NOT_EXPORTED',
  });
  assert.throws(() => import.meta.resolve('@microsoft/webui-framework/deferred.js'), {
    code: 'ERR_PACKAGE_PATH_NOT_EXPORTED',
  });
  t.mock.method(globalThis, 'fetch', async () => Response.json({
    ...record({ value: 'old' }), type: 'webui-deferred',
  }));
  await assert.rejects(createLazyLoader().fetchPartial(owner, id, { url: '/state' }), /mismatched lazy response/);
  assert.deepEqual(templates(), {});
});

test('concurrent preload shares metadata and configured modules without fetching state', async t => {
  let requests = 0;
  let modules = 0;
  t.mock.method(globalThis, 'fetch', async () => {
    requests++;
    return Response.json(record());
  });
  const loader = createLazyLoader({ loaders: { [fragment]: async () => { modules++; } } });
  await Promise.all([loader.ensureLoaded(owner, id), loader.ensureLoaded(owner, id)]);
  await loader.ensureLoaded(owner, id);
  assert.equal(requests, 1);
  assert.equal(modules, 1);
  assert.deepEqual(window.__webui?.state, {});
  assert.ok(templates()[fragment]);
});

test('fully warm requests still return independent fresh state', async t => {
  let states = 0;
  t.mock.method(globalThis, 'fetch', async (url: unknown, options?: RequestInit) => {
    if (String(url).includes('_webui/lazy')) return Response.json(record());
    assert.equal(new Headers(options?.headers).get('X-WebUI-Inventory'), '01');
    const warm = record({ revision: ++states });
    delete warm.templates;
    return Response.json(warm);
  });
  const loader = createLazyLoader();
  await loader.ensureLoaded(owner, id);
  const first = await loader.fetchPartial(owner, id, { url: '/state' });
  const second = await loader.fetchPartial(owner, id, { url: '/state' });
  assert.deepEqual(first, { revision: 1 });
  assert.deepEqual(second, { revision: 2 });
  assert.deepEqual(window.__webui?.state, {}, 'state is returned to its caller, not published globally');
});

test('an in-flight preload does not delay the backend state request', async t => {
  const metadata = Promise.withResolvers<Response>();
  let states = 0;
  t.mock.method(globalThis, 'fetch', async (url: unknown) => {
    if (String(url).includes('_webui/lazy')) return metadata.promise;
    states++;
    return Response.json(record({ ready: true }));
  });
  const loader = createLazyLoader();
  const preload = loader.ensureLoaded(owner, id);
  const state = await loader.fetchPartial(owner, id, { url: '/state' });
  assert.equal(states, 1);
  assert.deepEqual(state, { ready: true });
  metadata.resolve(Response.json(record()));
  await preload;
});

test('aborting one preload waiter does not cancel shared resource work', async t => {
  const response = Promise.withResolvers<Response>();
  let requestSignal: AbortSignal | null | undefined;
  t.mock.method(globalThis, 'fetch', async (_url: unknown, options?: RequestInit) => {
    requestSignal = options?.signal;
    return response.promise;
  });
  const loader = createLazyLoader();
  const controller = new AbortController();
  const cancelled = loader.ensureLoaded(owner, id, controller.signal);
  const other = loader.ensureLoaded(owner, id);
  controller.abort();
  await assert.rejects(cancelled, { name: 'AbortError' });
  assert.equal(requestSignal?.aborted, false);
  response.resolve(Response.json(record()));
  await other;
});

test('NDJSON prepares resources before state and releases a cancelled reader', async t => {
  let cancelled = false;
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(new TextEncoder().encode(`${JSON.stringify(record())}\n`));
    },
    cancel() { cancelled = true; },
  });
  t.mock.method(globalThis, 'fetch', async () => new Response(body, {
    headers: { 'content-type': 'application/x-ndjson' },
  }));
  const loader = createLazyLoader({ transport: lazyNdjson });
  const controller = new AbortController();
  const pending = loader.fetchPartial(owner, id, { url: '/state', signal: controller.signal });
  await new Promise(setImmediate);
  assert.ok(templates()[fragment]);
  controller.abort();
  await assert.rejects(pending, { name: 'AbortError' });
  assert.equal(cancelled, true);
  assert.equal(body.locked, false);
});

test('metadata-only EOF is not a successful empty state result', async t => {
  const response = new Response(`${JSON.stringify(record())}\n`, {
    headers: { 'content-type': 'application/x-ndjson' },
  });
  t.mock.method(globalThis, 'fetch', async () => response);
  const loader = createLazyLoader({ transport: lazyNdjson });
  await assert.rejects(loader.fetchPartial(owner, id, { url: '/state' }), /before its state record/);
  assert.equal(response.body?.locked, false);
  assert.deepEqual(window.__webui?.state, {});
});

test('the deadline also cancels a stalled module after the state body arrived', async t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  t.mock.method(globalThis, 'fetch', async () => Response.json(record({ ready: true })));
  const loader = createLazyLoader({
    timeoutMs: 50,
    loaders: { [fragment]: () => new Promise(() => {}) },
  });
  const pending = loader.fetchPartial(owner, id, { url: '/state' });
  await new Promise(setImmediate);
  t.mock.timers.tick(51);
  await assert.rejects(pending, { name: 'TimeoutError' });
});

test('a build mismatch fails before templates or inventory are mutated', async t => {
  t.mock.method(globalThis, 'fetch', async () => Response.json({ ...record({ ready: true }), buildId: 'other-build' }));
  await assert.rejects(createLazyLoader().fetchPartial(owner, id, { url: '/state' }), /different build/);
  assert.deepEqual(templates(), {});
  assert.equal(window.__webui?.inventory, '');
});

test('collection during a state request retries with fresh inventory and state', async t => {
  templates()[fragment] = record().templates![fragment];
  window.__webui!.inventory = '01';
  const firstResponse = Promise.withResolvers<Response>();
  let requests = 0;
  t.mock.method(globalThis, 'fetch', async (_url: unknown, options?: RequestInit) => {
    requests++;
    if (requests === 1) return firstResponse.promise;
    assert.equal(new Headers(options?.headers).get('X-WebUI-Inventory'), null);
    return Response.json(record({ revision: 2 }));
  });
  const pending = createLazyLoader().fetchPartial(owner, id, { url: '/state' });
  clearInventory();
  delete templates()[fragment];
  const old = record({ revision: 1 });
  delete old.templates;
  firstResponse.resolve(Response.json(old));
  assert.deepEqual(await pending, { revision: 2 });
  assert.equal(requests, 2);
});

test('invalid record identity, state, and inventory fail explicitly', async t => {
  const malformed = [
    { ...record({}), fragment: 'different' },
    { ...record({}), inventory: 'f' },
    { ...record({}), state: [] },
    { ...record({}), required: [] },
  ];
  t.mock.method(globalThis, 'fetch', async () => Response.json(malformed.shift()));
  for (let i = 0; i < 4; i++) {
    await assert.rejects(createLazyLoader().fetchPartial(owner, id, { url: '/state' }));
  }
});

test('a late incompatible document bootstrap fails before starting a request', async t => {
  window.__webui = Object.assign({ buildId: 'preloaded-build' }, { templates: {} });
  t.mock.method(document, 'getElementById', () => ({
    textContent: '{"buildId":"document-build","state":{},"templates":{}}',
    remove() {},
  }));
  const network = t.mock.method(globalThis, 'fetch', async () => Response.json(record()));
  await assert.rejects(createLazyLoader().ensureLoaded(owner, id), /different builds/);
  assert.equal(network.mock.callCount(), 0);
});
