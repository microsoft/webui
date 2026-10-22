// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { lazyBlockOwner, findLazyFragment, requireLazyFragment } from './lazy-contract.js';
import type { TemplateMeta } from './template-types.js';

test('the adapter resolves hostless bodies and local block ownership without defining a component', async () => {
  const previousWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
  const previousDocument = Object.getOwnPropertyDescriptor(globalThis, 'document');
  const previousCustomElements = Object.getOwnPropertyDescriptor(globalThis, 'customElements');
  const id = 'app-shell::lazy:settings';
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
    assert.equal(findLazyFragment(id, 'app-shell'), undefined);
    assert.throws(() => requireLazyFragment(id, 'app-shell'), /webui-framework\/lazy\.js/);
    await import('./lazy-entry.js');
    assert.equal(requireLazyFragment(id, 'app-shell'), meta);
    assert.equal(lazyBlockOwner(meta), meta);
    assert.equal(lazyBlockOwner(meta.b![0]), meta);
    window.__webui!.templates!['wrong-identity'] = meta;
    assert.throws(() => requireLazyFragment('wrong-identity', 'app-shell'), /Invalid fragment/);
    assert.throws(() => requireLazyFragment(id, 'different-owner'), /does not belong/);
    assert.throws(() => requireLazyFragment('missing', 'app-shell'), /not loaded/);
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
