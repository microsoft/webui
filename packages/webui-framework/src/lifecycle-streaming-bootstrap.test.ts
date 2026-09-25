// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';

test('streaming mode reserves completion when the application arrives before its coordinator', async () => {
  const originals = new Map(
    ['Document', 'document', 'window'].map((name) =>
      [name, Object.getOwnPropertyDescriptor(globalThis, name)] as const,
    ),
  );
  let queries = 0;
  class BrowserDocument extends EventTarget {
    readyState = 'loading';
    querySelector(): object {
      queries++;
      return {};
    }
  }
  const document = new BrowserDocument();
  const window = new EventTarget();
  let completions = 0;
  window.addEventListener('webui:hydration-complete', () => { completions++; });
  Object.defineProperties(globalThis, {
    Document: { value: BrowserDocument, configurable: true },
    document: { value: document, configurable: true },
    window: { value: window, configurable: true },
  });
  try {
    const lifecycle = await import('./lifecycle.js');
    lifecycle.hydrationStart();
    lifecycle.hydrationEnd();
    document.readyState = 'interactive';
    document.dispatchEvent(new Event('DOMContentLoaded'));
    assert.equal(completions, 0, 'parser readiness is not streaming completion');
    assert.equal(lifecycle.__getLifecycleStateForTests().streamingGateActive, true);
    lifecycle.beginStreamingGate();
    lifecycle.markBoundaryPending();
    lifecycle.markBoundaryCommitted(true);
    assert.equal(completions, 1);
    assert.equal(queries, 1, 'the existing mode detector is cached');
  } finally {
    for (const [name, descriptor] of originals) {
      if (descriptor) Object.defineProperty(globalThis, name, descriptor);
      else Reflect.deleteProperty(globalThis, name);
    }
  }
});
