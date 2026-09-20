// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import type { TemplateMeta } from './template-types.js';

test('one registry listener preserves lazy definition and resource readiness', async () => {
  const previousWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
  const target = new EventTarget();
  const browser = {
    __webui: {},
    addEventListener: target.addEventListener.bind(target),
    dispatchEvent: target.dispatchEvent.bind(target),
  };
  Object.defineProperty(globalThis, 'window', { value: browser, configurable: true });
  try {
    const {
      installTemplateDefinitionPreparation,
      installTemplateResourcePreparation,
      registerTemplateData,
    } = await import('./template-registry.js');
    const templates: Record<string, TemplateMeta> = {
      'late-static': { h: '<p></p>', th: 1 },
    };
    const ready = Promise.withResolvers<void>();
    let definitions = 0;
    let resources = 0;
    installTemplateDefinitionPreparation((received, names) => {
      assert.equal(received, templates);
      assert.deepEqual(names, ['late-static']);
      definitions++;
      return ready.promise;
    });

    registerTemplateData(templates);
    assert.equal(definitions, 0, 'streaming metadata alone must not demand a runtime');
    const waits: PromiseLike<unknown>[] = [];
    const announce = (): void => {
      target.dispatchEvent(new CustomEvent('webui:templates-registered', {
        detail: {
          templates,
          waitUntil: (promise: PromiseLike<unknown>): void => { waits.push(promise); },
        },
      }));
    };
    announce();
    assert.equal(definitions, 1);
    assert.equal(waits.length, 1);
    installTemplateResourcePreparation(() => { resources++; return undefined; });
    assert.equal(resources, 0, 'the new runtime must load before resource preparation');
    ready.resolve();
    await Promise.all(waits);
    assert.equal(resources, 1);

    installTemplateDefinitionPreparation(() => Promise.reject(new Error('definition load failed')));
    waits.length = 0;
    announce();
    await assert.rejects(Promise.all(waits), /definition load failed/);
    assert.equal(resources, 1, 'a rejected definition load cannot report resources ready');

    installTemplateDefinitionPreparation(null);
    installTemplateDefinitionPreparation(() => {
      throw new Error('installed hosts must not retain the bootstrap demand hook');
    });
    waits.length = 0;
    announce();
    assert.equal(resources, 2);
    assert.equal(waits.length, 0);
  } finally {
    if (previousWindow) Object.defineProperty(globalThis, 'window', previousWindow);
    else Reflect.deleteProperty(globalThis, 'window');
  }
});
