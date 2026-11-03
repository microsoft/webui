// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { installTemplateCache, collectTemplates } from './template-cache.js';
import { load, resolve } from './lazy-runtime.js';
import type { TemplateMeta } from './template-types.js';

test('collection during publication readiness cannot acknowledge a root or ordinary dependency', async () => {
  const browser = Object.assign(new EventTarget(), { __webui: {} as WebUIRuntimeGlobal });
  const saved = ['window', 'document', 'fetch', 'customElements'].map(name => [name, Object.getOwnPropertyDescriptor(globalThis, name)] as const);
  const id = 'epoch-owner::lazy:@0';
  const second = 'epoch-owner::lazy:@1';
  const child = 'epoch-child';
  let requests = 0;
  Object.defineProperty(globalThis, 'window', { configurable: true, value: browser });
  Object.defineProperty(globalThis, 'customElements', { configurable: true, value: { get: () => true } });
  Object.defineProperty(globalThis, 'document', {
    configurable: true,
    value: {
      readyState: 'complete',
      querySelector: () => null,
      getElementById: () => null,
      createTreeWalker: () => ({ nextNode: () => null }),
    },
  });
  Object.defineProperty(globalThis, 'fetch', {
    configurable: true,
    value: async (url: string) => {
      requests++;
      const root = url === 'shared.json' ? second : id;
      return Response.json({
        type: 'webui-pack', version: 1, fragment: root, inventory: '06',
        required: [root, child], bits: { [root]: 1, [child]: 2 },
        templates: {
          [root]: { v: 3, f: root, o: 'epoch-owner', e: 1, h: '<epoch-child></epoch-child>', d: [child] },
          [child]: { v: 3, h: '<p>child</p>' },
        },
      });
    },
  });
  installTemplateCache();
  let resume!: () => void;
  let published!: () => void;
  const publication = new Promise<void>(resolve => { published = resolve; });
  const held = new Promise<void>(resolve => { resume = resolve; });
  const listener = (event: Event): void => {
    const detail = (event as CustomEvent<{
      templates: Record<string, TemplateMeta>;
      waitUntil(ready: PromiseLike<unknown>): void;
    }>).detail;
    if (detail.templates[id]) { detail.waitUntil(held); published(); }
  };
  browser.addEventListener('webui:templates-registered', listener);
  try {
    const loading = load(id, 'epoch.json');
    await publication;
    collectTemplates();
    assert.equal(browser.__webui.templates?.[id], undefined);
    assert.equal(browser.__webui.templates?.[child], undefined);
    resume();
    await loading;
    assert.equal(browser.__webui.inventory, '', 'the old publication cannot resurrect root or child bits');
    browser.removeEventListener('webui:templates-registered', listener);
    const { setFlagsFromString } = await import('node:v8');
    const { runInNewContext } = await import('node:vm');
    setFlagsFromString('--expose-gc');
    await new Promise(done => setTimeout(done, 0));
    (runInNewContext('gc') as () => void)();
    assert.equal(resolve(id), undefined, 'completed installs do not permanently retain collected roots');
    assert.equal(resolve(child), undefined);
    await load(id, 'epoch.json');
    assert.ok(browser.__webui.templates?.[id], 'a new load safely restores availability');
    assert.ok(browser.__webui.templates?.[child]);
    assert.equal(browser.__webui.inventory, '06');
    assert.equal(requests, 2);
    const styles = Promise.withResolvers<void>();
    const secondPublication = Promise.withResolvers<void>();
    browser.addEventListener('webui:templates-registered', () => secondPublication.resolve(), { once: true });
    Reflect.set(browser, '__webuiPrepareResources', () => styles.promise);
    const existingChild = browser.__webui.templates?.[child];
    const first = load(second, 'shared.json');
    const joined = load(second, 'shared.json');
    await secondPublication.promise;
    collectTemplates([second]);
    assert.ok(browser.__webui.templates?.[second]);
    assert.equal(browser.__webui.templates?.[child], existingChild, 'the retained root owns its shared ordinary dependency');
    styles.resolve();
    await Promise.all([first, joined]);
    assert.equal(requests, 3, 'concurrent loads share one fetch even across style readiness and collection');
    assert.equal(browser.__webui.inventory, '', 'old bits are conservatively suppressed even if collection kept the roots');
    Reflect.deleteProperty(browser, '__webuiPrepareResources');
  } finally {
    resume();
    for (const [name, descriptor] of saved) {
      if (descriptor) Object.defineProperty(globalThis, name, descriptor);
      else Reflect.deleteProperty(globalThis, name);
    }
  }
});
