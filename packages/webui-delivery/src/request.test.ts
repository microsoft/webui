// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import './browser-shim.js';
import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { ResourceRequest, resourceHeaders } from './request.js';

test('resource request headers share build and both inventory identities', () => {
  const previous = window.__webui;
  try {
    window.__webui = { inventory: '01', styleInventory: '04', buildId: 'build-a' };
    assert.deepEqual(resourceHeaders('application/json'), {
      Accept: 'application/json',
      'X-WebUI-Inventory': '01',
      'X-WebUI-Style-Inventory': '04',
      'X-WebUI-Build': 'build-a',
    });
  } finally {
    window.__webui = previous;
  }
});

test('resource request cancellation preserves the reason and releases its deadline', t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const controller = new AbortController();
  const request = new ResourceRequest(controller.signal, 20);
  const reason = new DOMException('Superseded', 'AbortError');
  controller.abort(reason);
  assert.equal(request.signal.aborted, true);
  assert.equal(request.signal.reason, reason);
  t.mock.timers.tick(21);
  assert.equal(request.signal.reason, reason);
  request.dispose();
});

test('completed resource requests release their timer and external abort listener', t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const external = new AbortController();
  const request = new ResourceRequest(external.signal, 20);
  request.dispose();
  external.abort();
  t.mock.timers.tick(21);
  assert.equal(request.signal.aborted, false);
});

test('resource deadlines bound the entire request and reject invalid configuration', t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  for (const timeout of [0, -1, Infinity, NaN]) {
    assert.throws(() => new ResourceRequest(undefined, timeout), RangeError);
  }
  const request = new ResourceRequest(undefined, 20);
  t.mock.timers.tick(21);
  assert.equal(request.signal.reason.name, 'TimeoutError');
  const external = new AbortController();
  external.abort();
  assert.throws(() => new ResourceRequest(external.signal), { name: 'AbortError' });
});
