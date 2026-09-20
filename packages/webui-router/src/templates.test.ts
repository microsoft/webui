// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import './browser-shim.js';

import { strict as assert } from 'node:assert';
import { test } from 'node:test';

import { WebUIRouter } from './router.js';
import {
  readStreamingPartial,
  startDeferredStream,
  type StreamingContext,
} from './streaming.js';
import {
  registerTemplatesAndStyles,
  waitForTemplateReadiness,
} from './templates.js';

interface RegistrationDetail {
  templates: Record<string, unknown>;
  waitUntil(promise: PromiseLike<unknown>): void;
}

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
      new Set(),
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
    markCacheComplete() {},
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
    markCacheComplete() {},
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
    markCacheComplete() {},
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
    markCacheComplete() {},
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
    markCacheComplete() {},
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
      new Set(),
      () => {},
    ),
    undefined,
  );
});
