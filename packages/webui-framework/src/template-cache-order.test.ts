// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { getTemplate } from './template-registry.js';

test('late cache installation fails instead of guessing ownership of existing consumers', async () => {
  const previous = Object.getOwnPropertyDescriptor(globalThis, 'window');
  const meta = { v: 3, h: '<p>Already consumed</p>' };
  Object.defineProperty(globalThis, 'window', {
    configurable: true,
    value: { __webui: { templates: { 'x-used': meta } } },
  });
  try {
    assert.equal(getTemplate('x-used'), meta);
    await assert.rejects(import('./template-cache-entry.js'), /before component definitions/);
    assert.equal(window.__webui!.templates!['x-used'], meta);
  } finally {
    if (previous) Object.defineProperty(globalThis, 'window', previous);
    else Reflect.deleteProperty(globalThis, 'window');
  }
});
