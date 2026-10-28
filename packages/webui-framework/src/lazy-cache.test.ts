// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import type { CondBinding, TemplateInstance } from './element/types.js';
import type { TemplateMeta } from './template-types.js';

const id = 'cache-owner::lazy:@0';
const url = '_webui/packs/cache-owner.0001.json';
const idleCallbacks: (() => void)[] = [];
// Stands in for the template cache's weak records of every template read.
const cached = new Map<string, TemplateMeta>();
const browser = Object.assign(new EventTarget(), { __webui: { inventory: '00' } as NonNullable<Window['__webui']> });

Object.assign(globalThis, {
  window: browser,
  document: {
    readyState: 'complete',
    head: { appendChild(): void {} },
    createElement: () => ({ remove(): void {} }),
    querySelector: () => null,
    getElementById: () => null,
    addEventListener(): void {},
  },
  requestIdleCallback: (callback: () => void): number => idleCallbacks.push(callback),
  fetch: async (): Promise<Response> => new Response(JSON.stringify({
    type: 'webui-pack',
    version: 1,
    fragment: id,
    inventory: '02',
    styleInventory: '',
    required: [id],
    bits: { [id]: 1 },
    templates: { [id]: { v: 3, f: id, o: 'cache-owner', h: '<p></p>' } },
  }), { headers: { 'Content-Type': 'application/json' } }),
});

function releaseAtIdle(runtime: typeof import('./lazy-runtime.js'), binding: CondBinding): void {
  binding.instance = {} as TemplateInstance;
  runtime.opened(id, binding);
  binding.instance = null;
  runtime.disposed();
  for (const callback of idleCallbacks.splice(0)) callback();
}

test('a released region the template cache still holds is revived and tracked when it mounts', async () => {
  const { installTemplateCacheHooks } = await import('./template-cache-contract.js');
  installTemplateCacheHooks({
    accept(): void {},
    hydrated(): void {},
    visitRetained(): void {},
    retain(tag, meta) {
      cached.set(tag, meta);
      return meta;
    },
    lookup: tag => cached.get(tag),
  });
  const runtime = await import('./lazy-runtime.js');
  await runtime.load(id, url);
  const registry = browser.__webui.templates!;
  const template = registry[id];
  assert.ok(template);
  const owner = { conds: [] as CondBinding[] } as unknown as TemplateInstance;
  const binding = { blockIndex: id, owner, instance: null } as unknown as CondBinding;
  owner.conds.push(binding);

  releaseAtIdle(runtime, binding);
  assert.equal(registry[id], undefined);
  assert.equal(browser.__webui.inventory, '00');
  assert.equal(runtime.resolve(id, 'cache-owner'), template, 'a lookup may use the weak copy');
  assert.equal(registry[id], undefined, 'without registering it');

  assert.equal(runtime.resolve(id, 'cache-owner', true), template);
  assert.equal(registry[id], template, 'mounting registers the same metadata again');
  assert.equal(browser.__webui.inventory, '02');
  releaseAtIdle(runtime, binding);
  assert.equal(registry[id], undefined, 'the revived region is tracked and released again');
  assert.equal(browser.__webui.inventory, '00');
});
