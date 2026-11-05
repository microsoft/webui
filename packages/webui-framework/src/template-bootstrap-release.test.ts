// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import type { ComponentStyles } from './element/style-catalog.js';

test('hydration completion releases the non-routed bootstrap handoff once', async () => {
  const previousWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
  const previousDocument = Object.getOwnPropertyDescriptor(globalThis, 'document');
  const ownerDocument = {} as Document;
  const template = { h: '<p>Hello</p>' };
  const styles: ComponentStyles = {
    version: 1, strategy: 'style',
    resources: { greeting: { kind: 'style', css: 'p { color: red }' } },
    closures: { greeting: ['greeting'] },
  };
  const listeners = new Map<string, {
    listener: EventListener;
    options?: boolean | AddEventListenerOptions;
  }>();
  const fakeWindow = {
    __webui: {
      state: { title: 'Hello' },
      templates: { greeting: template },
      componentStyles: styles as ComponentStyles | undefined,
    },
    __webuiRegisterComponentStyles: (() => undefined) as ((value: unknown) => undefined) | undefined,
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
    Object.defineProperty(globalThis, 'document', { value: ownerDocument, configurable: true });

    const { installTemplateRegistry, releaseSSRBootstrapState } = await import('./template.js');
    assert.equal(listeners.size, 0, 'pure API imports do not install the registry');
    installTemplateRegistry();

    const registration = listeners.get('webui:hydration-complete');
    assert.ok(registration);
    assert.deepEqual(registration.options, { once: true });
    registration.listener.call(
      fakeWindow,
      new Event('webui:hydration-complete'),
    );
    assert.equal(fakeWindow.__webui.state, undefined);
    assert.equal(fakeWindow.__webui.templates.greeting, template);
    assert.equal(fakeWindow.__webui.componentStyles, undefined);
    const { catalogFor } = await import('./element/style-catalog.js');
    const catalog = catalogFor(ownerDocument);
    assert.deepEqual(catalog.resources.get('greeting'), styles.resources.greeting);
    assert.notEqual(catalog.resources.get('greeting'), styles.resources.greeting);
    assert.deepEqual(catalog.closures.get('greeting'), ['greeting']);

    fakeWindow.__webui.componentStyles = styles;
    fakeWindow.__webuiRegisterComponentStyles = undefined;
    releaseSSRBootstrapState();
    assert.equal(fakeWindow.__webui.componentStyles, styles, 'A resource-only fallback still owns its catalog');
  } finally {
    if (previousDocument) Object.defineProperty(globalThis, 'document', previousDocument);
    else Reflect.deleteProperty(globalThis, 'document');
    if (previousWindow) {
      Object.defineProperty(globalThis, 'window', previousWindow);
    } else {
      Reflect.deleteProperty(globalThis, 'window');
    }
  }
});
