// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';

import { getTemplate, registerTemplateData, type TemplateMeta } from './template.js';

describe('template registry helpers', () => {
  test('getTemplate returns registered metadata from window', () => {
    const previousWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
    const template: TemplateMeta = { h: '<p>Hello</p>' };

    try {
      Object.defineProperty(globalThis, 'window', {
        value: {
          __webui: {
            templates: {
              greeting: template,
            },
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

  test('registerTemplateData normalizes indexed conditions once', () => {
    const previousWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
    const fn = (): boolean => true;
    const template = {
      h: '<p></p>',
      c: [[[0, ['ready']], 0, [[], 0]]],
    } as unknown as TemplateMeta;

    try {
      Object.defineProperty(globalThis, 'window', {
        value: { __webui: {} },
        configurable: true,
        writable: true,
      });

      registerTemplateData({ greeting: template }, { greeting: [fn] });
      const registered = getTemplate('greeting')!;
      assert.equal((registered.c![0][0][0] as unknown), fn);
      assert.deepEqual(registered.c![0][0][1], ['ready']);
    } finally {
      if (previousWindow) {
        Object.defineProperty(globalThis, 'window', previousWindow);
      } else {
        Reflect.deleteProperty(globalThis, 'window');
      }
    }
  });

  test('getTemplate normalizes bootstrapped SSR metadata from window', () => {
    const previousWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
    const fn = (): boolean => false;
    const template = {
      h: '<p></p>',
      c: [[[0, ['ready']], 0, [[], 0]]],
    } as unknown as TemplateMeta;

    try {
      Object.defineProperty(globalThis, 'window', {
        value: {
          __webui: {
            inventory: '0c',
            state: { title: 'Hello' },
            templates: { greeting: template },
            templateFns: { greeting: [fn] },
          },
        },
        configurable: true,
        writable: true,
      });

      const registered = getTemplate('greeting')!;
      assert.equal((registered.c![0][0][0] as unknown), fn);
      assert.equal(window.__webui!.inventory, '0c');
      assert.deepEqual(window.__webui!.state, { title: 'Hello' });
    } finally {
      if (previousWindow) {
        Object.defineProperty(globalThis, 'window', previousWindow);
      } else {
        Reflect.deleteProperty(globalThis, 'window');
      }
    }
  });
});
