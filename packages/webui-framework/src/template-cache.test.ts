// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { getTemplate, registerTemplateData } from './template-registry.js';
import {
  collectTemplates,
  installTemplateCache,
  templateDependenciesForTests,
} from './template-cache.js';
import type { TemplateMeta } from './template-types.js';

function withCache(run: () => void): void {
  const previousWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
  const previousDocument = Object.getOwnPropertyDescriptor(globalThis, 'document');
  Object.defineProperty(globalThis, 'window', {
    configurable: true,
    value: Object.assign(new EventTarget(), {
      __webui: { templates: {}, state: {}, inventory: 'ff' },
    }),
  });
  Object.defineProperty(globalThis, 'document', {
    configurable: true,
    value: {
      nodeType: 9,
      getElementById: () => null,
      createTreeWalker: () => ({ nextNode: () => null }),
    },
  });
  try {
    installTemplateCache();
    run();
  } finally {
    if (previousWindow) Object.defineProperty(globalThis, 'window', previousWindow);
    else Reflect.deleteProperty(globalThis, 'window');
    if (previousDocument) Object.defineProperty(globalThis, 'document', previousDocument);
    else Reflect.deleteProperty(globalThis, 'document');
  }
}

test('cache release preserves the ordinary dependency graph of retained metadata', () => withCache(() => {
  const parent: TemplateMeta = { v: 3, h: '<x-child></x-child>', d: ['x-child'] };
  const child: TemplateMeta = { v: 3, h: '<x-leaf></x-leaf>', d: ['x-leaf'] };
  const leaf: TemplateMeta = { v: 3, h: '<span>Leaf</span>' };
  registerTemplateData({ 'x-parent': parent, 'x-child': child, 'x-leaf': leaf });

  assert.equal(collectTemplates(), 3);
  assert.deepEqual(Object.keys(window.__webui!.templates!), []);
  assert.equal(window.__webui!.inventory, '');
  assert.deepEqual(templateDependenciesForTests(parent), [child]);
  assert.deepEqual(templateDependenciesForTests(child), [leaf]);
  assert.equal(getTemplate('x-child'), child);
  assert.deepEqual(Object.keys(window.__webui!.templates!), [], 'owned lookup must not restore cache ownership');
}));

test('explicitly retained roots keep their dependency closure in the cache', () => withCache(() => {
  const parent: TemplateMeta = { v: 3, h: '<x-child></x-child>', d: ['x-child'] };
  const child: TemplateMeta = { v: 3, h: '<span></span>' };
  registerTemplateData({
    'x-parent': parent,
    'x-child': child,
    'x-unused': { v: 3, h: '<aside></aside>' },
  });
  assert.equal(collectTemplates(['x-parent']), 1);
  assert.deepEqual(Object.keys(window.__webui!.templates!).sort(), ['x-child', 'x-parent']);
  assert.equal(window.__webui!.templates!['x-child'], child);
}));

test('late dependency publication fills the retained graph without rebuilding a parent', () => withCache(() => {
  const parent: TemplateMeta = { v: 3, h: '<x-late></x-late>', d: ['x-late'] };
  const child: TemplateMeta = { v: 3, h: '<p>Late</p>' };
  registerTemplateData({ 'x-parent': parent });
  collectTemplates();
  registerTemplateData({ 'x-late': child });
  assert.deepEqual(templateDependenciesForTests(parent), [child]);
}));

test('repeated resource responses reuse canonical metadata', () => withCache(() => {
  const original: TemplateMeta = { v: 3, h: '<p>Stable</p>', tr: ['title'], ta: ['title'] };
  registerTemplateData({ 'x-card': original });
  collectTemplates();
  const duplicate: TemplateMeta = { v: 3, h: '<p>Stable</p>', tr: ['title'], ta: ['title'] };
  registerTemplateData({ 'x-card': duplicate });
  assert.equal(getTemplate('x-card'), original);
  assert.equal(window.__webui!.templates!['x-card'], original);
}));

test('conflicting batches fail before publishing any new template', () => withCache(() => {
  const original: TemplateMeta = { v: 3, h: '<p>Original</p>' };
  registerTemplateData({ 'x-card': original });
  assert.throws(() => registerTemplateData({
    'x-new': { v: 3, h: '<p>New</p>' },
    'x-card': { v: 3, h: '<p>Different artifact</p>' },
  }), /conflicts with a live compiled template/);
  assert.equal(window.__webui!.templates!['x-new'], undefined);
  assert.equal(getTemplate('x-new'), undefined);
  assert.equal(getTemplate('x-card'), original);
}));

test('legacy metadata fails before collection mutates the cache or inventory', () => withCache(() => {
  const old: TemplateMeta = JSON.parse('{"h":"<p>Invalid format</p>"}');
  window.__webui!.templates!['x-old'] = old;
  assert.throws(() => collectTemplates(), /has no cache ownership metadata/);
  assert.equal(window.__webui!.templates!['x-old'], old);
  assert.equal(window.__webui!.inventory, 'ff');
}));

test('invalid dependency metadata is rejected before publication', () => withCache(() => {
  const invalid: TemplateMeta = JSON.parse('{"v":3,"h":"","d":[null]}');
  assert.throws(() => registerTemplateData({ 'x-invalid': invalid }), /Invalid component dependencies/);
  assert.equal(window.__webui!.templates!['x-invalid'], undefined);
}));

test('resource readiness can reclaim a retained template after explicit collection', () => withCache(() => {
  const meta: TemplateMeta = { v: 3, h: '<p>Reusable</p>' };
  registerTemplateData({ 'x-card': meta });
  collectTemplates();
  assert.equal(window.__webuiRetainTemplate?.('x-card'), meta);
  assert.equal(window.__webui!.templates!['x-card'], meta);
}));

test('cyclic dependency input remains bounded during an explicit keep walk', () => withCache(() => {
  registerTemplateData({
    'x-a': { v: 3, h: '', d: ['x-b'] },
    'x-b': { v: 3, h: '', d: ['x-a'] },
  });
  assert.equal(collectTemplates(['x-a']), 0);
  assert.deepEqual(Object.keys(window.__webui!.templates!).sort(), ['x-a', 'x-b']);
}));
