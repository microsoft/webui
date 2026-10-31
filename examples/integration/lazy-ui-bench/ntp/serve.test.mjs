// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from 'node:assert/strict';
import { get } from 'node:http';
import { test } from 'node:test';
import { serve } from './serve.mjs';

async function fixtureServer(t) {
  const fixtures = new Map([['fixture', {
    protocol: { render: () => '<!doctype html><html><body>Fixture</body></html>' },
    files: new Map(),
  }]]);
  const server = await serve(fixtures, {}, Buffer.from('image'));
  t.after(() => server.close());
  return `${server.origin}/fixture/`;
}

test('the benchmark server preserves valid load holds and the 5000 ms boundary', async t => {
  const origin = await fixtureServer(t);
  const ordinary = await fetch(origin);
  assert.equal(ordinary.status, 200);
  assert.doesNotMatch(await ordinary.text(), /__hold/);

  const held = await fetch(`${origin}?hold=5000`);
  assert.equal(held.status, 200);
  assert.match(await held.text(), /__hold\.gif\?ms=5000/);

  for (const milliseconds of [0, 10]) {
    const response = await fetch(`${origin}__hold.gif?ms=${milliseconds}`);
    assert.equal(response.status, 200);
    assert.equal(response.headers.get('content-type'), 'image/gif');
    assert.equal(Buffer.from(await response.arrayBuffer()).subarray(0, 6).toString(), 'GIF89a');
  }
});

test('the benchmark server rejects invalid or unbounded load holds', async t => {
  const origin = await fixtureServer(t);
  for (const path of ['?hold=', '__hold.gif?ms=']) {
    for (const value of ['-1', '1.5', 'NaN', 'Infinity', '5001']) {
      await t.test(`${path}${value}`, async () => {
        const response = await fetch(`${origin}${path}${value}`, {
          signal: AbortSignal.timeout(1500),
        });
        assert.equal(response.status, 400);
        assert.match(await response.text(), /0.*5000/);
      });
    }
  }
});

test('disconnecting a held request cancels its pending timer', { timeout: 2000 }, async t => {
  const origin = await fixtureServer(t);
  const setTimer = globalThis.setTimeout;
  const clearTimer = globalThis.clearTimeout;
  let heldTimer;
  let scheduled;
  let cancelled;
  const whenScheduled = new Promise(resolve => { scheduled = resolve; });
  const whenCancelled = new Promise(resolve => { cancelled = resolve; });
  t.mock.method(globalThis, 'setTimeout', (callback, duration, ...args) => {
    const timer = setTimer(callback, duration, ...args);
    if (duration === 5000) {
      heldTimer = timer;
      scheduled();
    }
    return timer;
  });
  t.mock.method(globalThis, 'clearTimeout', timer => {
    clearTimer(timer);
    if (timer === heldTimer) cancelled();
  });
  const request = get(`${origin}__hold.gif?ms=5000`);
  request.on('error', error => assert.equal(error.code, 'ECONNRESET'));
  t.after(() => {
    request.destroy();
    clearTimer(heldTimer);
  });
  await whenScheduled;
  request.destroy();
  await whenCancelled;
});
