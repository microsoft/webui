// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import './browser-shim.js';
import { strict as assert } from 'node:assert';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import type { RouterConfig } from '@microsoft/webui-delivery/router.js';
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
