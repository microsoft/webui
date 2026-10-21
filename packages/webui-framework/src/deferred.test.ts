// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { deferredBlockOwner, findDeferredFragment, requireDeferredFragment } from './deferred-contract.js';
import type { TemplateMeta } from './template-types.js';

test('the adapter resolves hostless bodies and local block ownership without defining a component', async () => {
  const previousWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
  const previousDocument = Object.getOwnPropertyDescriptor(globalThis, 'document');
  const previousCustomElements = Object.getOwnPropertyDescriptor(globalThis, 'customElements');
  const id = 'app-shell::defer:settings';
  const meta: TemplateMeta = {
    v: 3, f: id, o: 'app-shell', e: 1, h: '<p></p>',
    b: [{ h: '<span></span>' }],
  };
  Object.defineProperty(globalThis, 'window', {
    configurable: true,
    value: Object.assign(new EventTarget(), { __webui: { templates: { [id]: meta } } }),
  });
  Object.defineProperty(globalThis, 'document', {
    configurable: true, value: { getElementById: () => null },
  });
  Object.defineProperty(globalThis, 'customElements', {
    configurable: true,
    get: () => { throw new Error('hostless fragments must not use the custom-element registry'); },
  });
  try {
    assert.equal(findDeferredFragment(id, 'app-shell'), undefined);
    assert.throws(() => requireDeferredFragment(id, 'app-shell'), /webui-framework\/deferred\.js/);
    await import('./deferred-entry.js');
    assert.equal(requireDeferredFragment(id, 'app-shell'), meta);
    assert.equal(deferredBlockOwner(meta.b![0]), meta);
    assert.throws(() => requireDeferredFragment(id, 'different-owner'), /does not belong/);
    assert.throws(() => requireDeferredFragment('missing', 'app-shell'), /not loaded/);
    assert.deepEqual(window.__webuiResolveResources?.(id), [id]);
  } finally {
    for (const [name, descriptor] of [
      ['window', previousWindow], ['document', previousDocument], ['customElements', previousCustomElements],
    ] as const) {
      if (descriptor) Object.defineProperty(globalThis, name, descriptor);
      else Reflect.deleteProperty(globalThis, name);
    }
  }
});
