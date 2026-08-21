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
      b: [{ h: '<span></span>' }],
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

  test('registerTemplateData does not re-announce styles it already registered', () => {
    const previousWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
    const previousDocument = Object.getOwnPropertyDescriptor(globalThis, 'document');
    let announcedStyles: unknown = 'not-dispatched';

    try {
      Object.defineProperty(globalThis, 'window', {
        value: {
          __webui: {},
          dispatchEvent(event: Event): boolean {
            announcedStyles = (event as CustomEvent<{
              componentStyles?: unknown;
            }>).detail.componentStyles;
            return true;
          },
        },
        configurable: true,
        writable: true,
      });
      Object.defineProperty(globalThis, 'document', {
        value: { nodeType: 9 },
        configurable: true,
        writable: true,
      });

      registerTemplateData(
        { 'style-card': { h: '<p>Styled</p>' } },
        undefined,
        {
          version: 1,
          strategy: 'style',
          resources: {
            'style-card': { kind: 'style', css: '.style-card{}' },
          },
          closures: { 'style-card': ['style-card'] },
        },
      );

      assert.equal(announcedStyles, undefined);
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

  test('getTemplate normalizes bootstrapped SSR metadata from window', () => {
    const previousWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
    const fn = (): boolean => false;
    const template = {
      h: '<p></p>',
      b: [{ h: '<span></span>' }],
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

  test('getTemplate lazily loads webui-data and preserves eager runtime metadata', () => {
    const previousWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
    const previousDocument = Object.getOwnPropertyDescriptor(globalThis, 'document');
    const fn = (): boolean => true;
    let removed = false;

    try {
      Object.defineProperty(globalThis, 'window', {
        value: {
          __webui: {
            componentAssetStyles: { 'lazy-panel': ['/lazy-panel.css'] },
            templateFns: { greeting: [fn] },
          },
        },
        configurable: true,
        writable: true,
      });
      Object.defineProperty(globalThis, 'document', {
        value: {
          getElementById(id: string) {
            if (id !== 'webui-data') return null;
            return {
              textContent: '{"inventory":"0c","state":{"title":"Hello"},"componentStyles":{"version":1,"strategy":"style","resources":{},"closures":{}},"templates":{"greeting":{"h":"<p></p>","b":[{"h":"<span></span>"}],"c":[[[0,["ready"]],0,[[],0]]]}}}',
              remove() { removed = true; },
            };
          },
        },
        configurable: true,
        writable: true,
      });

      const registered = getTemplate('greeting')!;
      assert.equal((registered.c![0][0][0] as unknown), fn);
      assert.equal(window.__webui!.inventory, '0c');
      assert.deepEqual(window.__webui!.state, { title: 'Hello' });
      assert.deepEqual(
        window.__webui!.componentAssetStyles,
        { 'lazy-panel': ['/lazy-panel.css'] },
      );
      assert.equal(removed, true);
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
