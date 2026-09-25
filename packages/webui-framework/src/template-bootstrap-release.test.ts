// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';

test('hydration completion releases the non-routed bootstrap handoff once', async () => {
  const previousWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
  const template = { h: '<p>Hello</p>' };
  const listeners = new Map<string, {
    listener: EventListener;
    options?: boolean | AddEventListenerOptions;
  }>();
  const fakeWindow = {
    __webui: {
      state: { title: 'Hello' },
      templates: { greeting: template },
    },
    addEventListener(
      type: string,
      listener: EventListener,
      options?: boolean | AddEventListenerOptions,
    ): void {
      listeners.set(type, { listener, options });
    },
  };

  try {
    Object.defineProperty(globalThis, 'window', {
      value: fakeWindow,
      configurable: true,
      writable: true,
    });

    await import('./template.js');

    const registration = listeners.get('webui:hydration-complete');
    assert.ok(registration);
    assert.deepEqual(registration.options, { once: true });
    registration.listener.call(
      fakeWindow,
      new Event('webui:hydration-complete'),
    );
    assert.equal(fakeWindow.__webui.state, undefined);
    assert.equal(fakeWindow.__webui.templates.greeting, template);
  } finally {
    if (previousWindow) {
      Object.defineProperty(globalThis, 'window', previousWindow);
    } else {
      Reflect.deleteProperty(globalThis, 'window');
    }
  }
});
