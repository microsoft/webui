// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';

test('the API barrel does not inspect the document or install runtime work', async t => {
  const saved = ['HTMLElement', 'window', 'document'].map(name =>
    [name, Object.getOwnPropertyDescriptor(globalThis, name)] as const);
  let timers = 0;
  t.mock.method(globalThis, 'setTimeout', () => {
    timers++;
    throw new Error('Pure imports must not schedule runtime work');
  });
  Object.defineProperties(globalThis, {
    HTMLElement: { value: class {}, configurable: true },
    window: {
      configurable: true,
      get() { throw new Error('Pure imports must not install window hooks'); },
    },
    document: {
      configurable: true,
      get() { throw new Error('Pure imports must not inspect the document'); },
    },
  });
  try {
    const api = await import('./index.js');
    assert.equal(typeof api.observable, 'function');
    assert.equal(typeof api.attr, 'function');
    assert.equal(typeof api.WebUIElement, 'function');
    const runtime = await import('./runtime.js');
    assert.equal(typeof runtime.startRuntime, 'function');
    assert.equal(timers, 0);
  } finally {
    for (const [name, descriptor] of saved) {
      if (descriptor) Object.defineProperty(globalThis, name, descriptor);
      else Reflect.deleteProperty(globalThis, name);
    }
  }
});
