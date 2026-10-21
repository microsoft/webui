// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';

test('synchronous definition errors reject transport readiness instead of escaping event dispatch', async () => {
  const previous = Object.getOwnPropertyDescriptor(globalThis, 'window');
  Object.defineProperty(globalThis, 'window', {
    configurable: true,
    value: Object.assign(new EventTarget(), { __webui: { templates: {} } }),
  });
  try {
    const { deferTemplateDefinition } = await import('./template-registry.js');
    const failure = new Error('definition could not be completed');
    deferTemplateDefinition('failing-definition', class {}, () => { throw failure; });
    const templates = { 'failing-definition': { h: '<p></p>' } };
    window.__webuiPrepareTemplates?.(templates);
    const waits: PromiseLike<unknown>[] = [];
    window.dispatchEvent(new CustomEvent('webui:templates-registered', {
      detail: {
        templates,
        waitUntil: (promise: PromiseLike<unknown>) => { waits.push(promise); },
      },
    }));
    assert.equal(waits.length, 1);
    await assert.rejects(Promise.all(waits), failure);
    assert.throws(
      () => window.__webuiPrepareTemplates?.({ 'invalid-template': { h: 5 } }),
      /Invalid compiled template/,
    );
  } finally {
    if (previous) Object.defineProperty(globalThis, 'window', previous);
    else Reflect.deleteProperty(globalThis, 'window');
  }
});
