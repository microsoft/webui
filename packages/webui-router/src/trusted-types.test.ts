// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { enforceTrustedTypesForTest } from './browser-shim.js';

import { strict as assert } from 'node:assert';
import { afterEach, beforeEach, test } from 'node:test';
import { WebUIRouter } from './router.js';
import { registerTemplatesAndStyles, TrustedTypesPayloadError } from './templates.js';
import type { RawPreloadedPartial } from './prepared-preload.js';

const originalFetch = globalThis.fetch;
const originalRuntime = window.__webui;
const originalStyles = window.__webuiRegisterComponentStyles;
const originalAppend = document.head.appendChild;
const failure = /Trusted Types.*condition-source.*full document navigation/;
let appended = 0;
let stylesRegistered = 0;
let restoreSinks: () => void;

beforeEach(() => {
  restoreSinks = enforceTrustedTypesForTest();
  window.__webui = { inventory: 'before', nonce: 'page-nonce' };
  appended = 0;
  stylesRegistered = 0;
  document.head.appendChild = node => {
    appended++;
    return node;
  };
  window.__webuiRegisterComponentStyles = () => {
    stylesRegistered++;
    return undefined;
  };
});

afterEach(() => {
  globalThis.fetch = originalFetch;
  document.head.appendChild = originalAppend;
  window.__webui = originalRuntime;
  window.__webuiRegisterComponentStyles = originalStyles;
  restoreSinks();
});

function unsupportedPayload() {
  return {
    path: '/next',
    chain: [{ component: 'source-card', path: '/next' }],
    templates: { 'source-card': { h: '<p>Next</p>' } },
    templateFunctions: { 'source-card': '[function(){return true}]' },
    componentStyles: {
      version: 1 as const,
      strategy: 'link' as const,
      resources: { card: { kind: 'link' as const, href: '/new.css' } },
      closures: { 'source-card': ['card'] },
    },
    inventory: 'after',
    css: ['/new.css'],
  };
}

function assertUnchanged(): void {
  assert.equal(appended, 0);
  assert.equal(stylesRegistered, 0);
  assert.deepEqual(window.__webui, { inventory: 'before', nonce: 'page-nonce' });
}

test('native enforcement rejects condition source before any response publication without a marker', () => {
  let inventoryChanged = false;
  assert.throws(() => registerTemplatesAndStyles(
    unsupportedPayload(),
    'page-nonce',
    () => { inventoryChanged = true; },
  ), failure);
  assert.equal(inventoryChanged, false);
  assertUnchanged();
});

test('native enforcement rejects source even without template metadata', () => {
  assert.throws(() => registerTemplatesAndStyles({
    templateFunctions: unsupportedPayload().templateFunctions,
    inventory: 'after',
  }, 'page-nonce', () => assert.fail('inventory must not change')), failure);
  assertUnchanged();
});

test('native rejection preserves the browser TypeError as its cause', () => {
  assert.throws(() => registerTemplatesAndStyles(
    unsupportedPayload(), 'page-nonce', () => {},
  ), error => error instanceof TrustedTypesPayloadError
    && error.cause instanceof TypeError
    && error.cause.message.includes('TrustedScript'));
  assertUnchanged();
});

test('native enforcement rejects empty source entries without publishing a wrapper script', () => {
  assert.throws(() => registerTemplatesAndStyles({
    templateFunctions: { 'empty-card': '' },
    inventory: 'after',
  }, 'page-nonce', () => assert.fail('inventory must not change')), failure);
  assertUnchanged();
});

test('native enforcement rejects FAST strings before publishing resources', () => {
  const payload = {
    ...unsupportedPayload(),
    templates: { 'fast-card': '<f-template>Untrusted</f-template>' },
    templateFunctions: {},
  };
  assert.throws(
    () => registerTemplatesAndStyles(payload, '', () => {}),
    /Trusted Types.*FAST\/string templates.*full document navigation/,
  );
  assertUnchanged();
});

interface RouterRequests {
  fetchPartial(path: string): Promise<unknown>;
  takePreparedPartial(path: string): Promise<unknown>;
  preparedPreload: {
    take(): Promise<RawPreloadedPartial>;
    release(path: string): void;
  };
}

for (const streaming of [false, true]) {
  test(`native enforcement during ${streaming ? 'NDJSON' : 'JSON'} navigation surfaces rejection without publication`, async () => {
    let cancelled = false;
    let requestMode: RequestMode | undefined;
    const payload = unsupportedPayload();
    const response = streaming
      ? new Response(new ReadableStream<Uint8Array>({
        start(controller) {
          controller.enqueue(new TextEncoder().encode(`${JSON.stringify(payload)}\n`));
        },
        cancel() { cancelled = true; },
      }), { headers: { 'Content-Type': 'application/x-ndjson' } })
      : new Response(JSON.stringify(payload), {
        headers: { 'Content-Type': 'application/json' },
      });
    globalThis.fetch = async (_input, init) => {
      requestMode = init?.mode;
      return response;
    };
    const router = new WebUIRouter() as unknown as RouterRequests;
    await assert.rejects(router.fetchPartial('/next'), failure);
    assert.equal(requestMode, 'same-origin');
    if (streaming) {
      assert.equal(cancelled, true);
      assert.equal(response.body?.locked, false);
    }
    assertUnchanged();
  });
}

test('native enforcement during prepared navigation releases the preload and surfaces rejection', async () => {
  let released: string | undefined;
  const router = new WebUIRouter() as unknown as RouterRequests;
  router.preparedPreload = {
    take: async () => ({
      controller: new AbortController(),
      inventory: 'before',
      timestamp: Date.now(),
      response: new Response(JSON.stringify(unsupportedPayload()), {
        headers: { 'Content-Type': 'application/json' },
      }),
    }),
    release(path) { released = path; },
  };
  await assert.rejects(router.takePreparedPartial('/next'), failure);
  assert.equal(released, '/next');
  assertUnchanged();
});
