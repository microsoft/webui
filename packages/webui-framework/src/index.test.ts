// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';

test('framework root import installs static hosts', async () => {
  const previousWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
  const previousDocument = Object.getOwnPropertyDescriptor(globalThis, 'document');
  const previousHTMLElement = Object.getOwnPropertyDescriptor(globalThis, 'HTMLElement');
  const previousCustomElements = Object.getOwnPropertyDescriptor(globalThis, 'customElements');
  const listeners = new Map<string, Array<(event: Event) => void>>();
  const registry = new Map<string, CustomElementConstructor>();

  try {
    Object.defineProperty(globalThis, 'HTMLElement', {
      value: class HTMLElement {},
      configurable: true,
    });
    Object.defineProperty(globalThis, 'customElements', {
      value: {
        get(name: string): CustomElementConstructor | undefined {
          return registry.get(name);
        },
        define(name: string, ctor: CustomElementConstructor): void {
          registry.set(name, ctor);
        },
      },
      configurable: true,
    });
    Object.defineProperty(globalThis, 'document', {
      value: {
        readyState: 'complete',
        getElementById() {
          return null;
        },
      },
      configurable: true,
    });
    Object.defineProperty(globalThis, 'window', {
      value: {
        __webui: {
          templates: {
            'html-only-card': {
              h: '<p></p>',
              th: 1,
              tr: ['title'],
              ta: ['title'],
            },
          },
        },
        addEventListener(type: string, listener: (event: Event) => void) {
          const existing = listeners.get(type);
          if (existing) {
            existing.push(listener);
          } else {
            listeners.set(type, [listener]);
          }
        },
      },
      configurable: true,
    });

    await import('./index.js');
    await new Promise<void>((resolve) => queueMicrotask(resolve));

    assert.equal(registry.has('html-only-card'), true);
    assert.equal(listeners.has('webui:templates-registered'), true);
  } finally {
    restoreDescriptor('window', previousWindow);
    restoreDescriptor('document', previousDocument);
    restoreDescriptor('HTMLElement', previousHTMLElement);
    restoreDescriptor('customElements', previousCustomElements);
  }
});

function restoreDescriptor(key: keyof typeof globalThis, descriptor: PropertyDescriptor | undefined): void {
  if (descriptor) {
    Object.defineProperty(globalThis, key, descriptor);
  } else {
    Reflect.deleteProperty(globalThis, key);
  }
}
