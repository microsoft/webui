// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { applyBoundaryBootstrap } from './streaming-bootstrap.js';
import { getTemplate } from './template-registry.js';

test('progressive resource identity is checked before templates or state can mutate', () => {
  const previous = Object.getOwnPropertyDescriptor(globalThis, 'window');
  const runtime = { buildId: 'document-build', state: { kept: true }, templates: {} };
  Object.defineProperty(globalThis, 'window', {
    configurable: true,
    value: Object.assign(new EventTarget(), { __webui: runtime }),
  });

  test('a late initial bootstrap cannot replace resources from another build', () => {
    const previousWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
    const previousDocument = Object.getOwnPropertyDescriptor(globalThis, 'document');
    const runtime = { buildId: 'preloaded-build', templates: {} };
    Object.defineProperty(globalThis, 'window', {
      configurable: true, value: Object.assign(new EventTarget(), { __webui: runtime }),
    });
    Object.defineProperty(globalThis, 'document', {
      configurable: true,
      value: {
        getElementById: () => ({
          textContent: '{"buildId":"document-build","state":{},"templates":{}}',
          remove() { throw new Error('An incompatible bootstrap must not be consumed.'); },
        }),
      },
    });
    try {
      assert.throws(() => getTemplate('unloaded-widget'), /different builds/);
      assert.equal(window.__webui, runtime);
    } finally {
      if (previousWindow) Object.defineProperty(globalThis, 'window', previousWindow);
      else Reflect.deleteProperty(globalThis, 'window');
      if (previousDocument) Object.defineProperty(globalThis, 'document', previousDocument);
      else Reflect.deleteProperty(globalThis, 'document');
    }
  });
  try {
    assert.throws(() => applyBoundaryBootstrap({
      buildId: 'different-build',
      declarationId: 0,
      templates: { 'unexpected-widget': { h: '<p>Unexpected</p>' } },
      componentStyles: { version: 1, strategy: 'style', resources: {}, closures: {} },
    }), /different document build/);
    assert.deepEqual(runtime.templates, {});
    assert.deepEqual(runtime.state, { kept: true });
    assert.equal(runtime.buildId, 'document-build');
  } finally {
    if (previous) Object.defineProperty(globalThis, 'window', previous);
    else Reflect.deleteProperty(globalThis, 'window');
  }
});
