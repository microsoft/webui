// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';

import { isStreamingHydrationMode, resetStreamingModeForTests } from './streaming-mode.js';

function withDocument<T>(meta: string | null, run: () => T): T {
  const previousDocument = Object.getOwnPropertyDescriptor(globalThis, 'document');
  let queries = 0;
  Object.defineProperty(globalThis, 'document', {
    value: {
      querySelector(selector: string) {
        queries++;
        assert.equal(selector, 'meta[name="webui-streaming"][content="1"]');
        return meta ? { getAttribute: () => meta } : null;
      },
    },
    configurable: true,
    writable: true,
  });
  try {
    return run();
  } finally {
    if (previousDocument) {
      Object.defineProperty(globalThis, 'document', previousDocument);
    } else {
      Reflect.deleteProperty(globalThis, 'document');
    }
    assert.ok(queries <= 1, `expected at most one document.querySelector call, got ${queries}`);
  }
}

describe('streaming-mode detection', () => {
  test('detects streaming mode from the meta tag', () => {
    resetStreamingModeForTests();
    withDocument('1', () => {
      assert.equal(isStreamingHydrationMode(), true);
    });
  });

  test('defaults to non-streaming when the meta tag is absent', () => {
    resetStreamingModeForTests();
    withDocument(null, () => {
      assert.equal(isStreamingHydrationMode(), false);
    });
  });

  test('caches the result across calls — queries the document at most once', () => {
    resetStreamingModeForTests();
    withDocument('1', () => {
      assert.equal(isStreamingHydrationMode(), true);
      assert.equal(isStreamingHydrationMode(), true);
      assert.equal(isStreamingHydrationMode(), true);
    });
  });

  test('resetStreamingModeForTests forces a fresh detection', () => {
    resetStreamingModeForTests();
    withDocument('1', () => {
      assert.equal(isStreamingHydrationMode(), true);
    });
    resetStreamingModeForTests();
    withDocument(null, () => {
      assert.equal(isStreamingHydrationMode(), false);
    });
  });
});
