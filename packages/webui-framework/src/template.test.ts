// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';

import { getTemplate, type TemplateMeta } from './template.js';

describe('template registry helpers', () => {
  test('getTemplate returns registered metadata from window', () => {
    const previousWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
    const template: TemplateMeta = { h: '<p>Hello</p>' };

    try {
      Object.defineProperty(globalThis, 'window', {
        value: {
          __webui_templates: {
            greeting: template,
          },
        },
        configurable: true,
        writable: true,
      });

      assert.equal(getTemplate('greeting'), template);
      assert.equal(getTemplate('missing'), undefined);
    } finally {
      if (previousWindow) {
        Object.defineProperty(globalThis, 'window', previousWindow);
      } else {
        Reflect.deleteProperty(globalThis, 'window');
      }
    }
  });
});
