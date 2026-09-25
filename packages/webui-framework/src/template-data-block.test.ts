// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { describe, test } from 'node:test';
import assert from 'node:assert/strict';

import { getTemplate } from './template.js';

// `loadWebUIDataBlock` latches after its first run, and that latch is module
// state. This lives in its own file so the latch starts clean; `node --test`
// gives every test file its own process.
describe('SSR data block loading', () => {
  test('publishes parsed data even when componentStyles are rejected', () => {
    const previousWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
    const previousDocument = Object.getOwnPropertyDescriptor(globalThis, 'document');
    let parses = 0;
    let removed = 0;

    try {
      Object.defineProperty(globalThis, 'window', {
        value: {},
        configurable: true,
        writable: true,
      });
      Object.defineProperty(globalThis, 'document', {
        value: {
          getElementById(id: string) {
            if (id !== 'webui-data') return null;
            return {
              get textContent(): string {
                parses++;
                // `componentStyles` is version 2, which registration rejects.
                return '{"state":{"title":"Hello"},"componentStyles":{"version":2},"templates":{"greeting":{"h":"<p></p>"}}}';
              },
              remove() { removed++; },
            };
          },
        },
        configurable: true,
        writable: true,
      });

      assert.throws(() => getTemplate('greeting'), /componentStyles must use version 1/);

      assert.deepEqual(
        window.__webui!.state,
        { title: 'Hello' },
        'state that parsed correctly must survive a style registration failure',
      );
      assert.ok(
        window.__webui!.templates!.greeting,
        'templates that parsed correctly must survive a style registration failure',
      );

      assert.equal(getTemplate('greeting')?.h, '<p></p>');
      assert.equal(parses, 1, 'a failed style registration must not re-parse the data block');
      assert.equal(removed, 1);
    } finally {
      if (previousWindow) {
        Object.defineProperty(globalThis, 'window', previousWindow);
      } else {
        Reflect.deleteProperty(globalThis, 'window');
      }
      if (previousDocument) {
        Object.defineProperty(globalThis, 'document', previousDocument);
      } else {
        Reflect.deleteProperty(globalThis, 'document');
      }
    }
  });
});
