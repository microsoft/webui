// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';

import { getTemplate, type TemplateMeta } from './template.js';
import { defineComponentAssets } from './component-asset.js';

type GlobalName = 'window' | 'document';

interface ScriptMock {
  type: string;
  nonce: string;
  textContent: string;
}

function setGlobal(name: GlobalName, value: unknown): PropertyDescriptor | undefined {
  const previous = Object.getOwnPropertyDescriptor(globalThis, name);
  Object.defineProperty(globalThis, name, {
    value,
    configurable: true,
    writable: true,
  });
  return previous;
}

function restoreGlobal(name: GlobalName, previous: PropertyDescriptor | undefined): void {
  if (previous) {
    Object.defineProperty(globalThis, name, previous);
  } else {
    Reflect.deleteProperty(globalThis, name);
  }
}

function assetModule(source: string): string {
  return `data:text/javascript,${encodeURIComponent(`export default ${source};`)}`;
}

function assetObjectModule(asset: unknown): string {
  return assetModule(JSON.stringify(asset));
}

function componentAsset(templates: Record<string, TemplateMeta>): Record<string, unknown> {
  return {
    type: 'webui-component-asset',
    version: 1,
    components: Object.keys(templates),
    templates,
  };
}

describe('component asset helpers', () => {
  test('manifest load registers templates and injects nonce importmaps', async () => {
    const appended: ScriptMock[] = [];
    const template: TemplateMeta = { h: '<p>Lazy</p>' };
    const previousWindow = setGlobal('window', { __webui: { nonce: 'abc123' } });
    const previousDocument = setGlobal('document', {
      baseURI: 'https://example.test/app/',
      createElement(tag: string) {
        assert.equal(tag, 'script');
        return { type: '', nonce: '', textContent: '' };
      },
      head: {
        appendChild(script: ScriptMock) {
          appended.push(script);
          return script;
        },
      },
      getElementById() {
        return null;
      },
      querySelector() {
        return null;
      },
    });

    try {
      const assets = defineComponentAssets({
        'lazy-card': {
          asset: assetObjectModule({
            ...componentAsset({ 'lazy-card': template }),
            templateStyles: [
              '<script type="importmap">{"imports":{"lazy-card":"data:text/css,body%7B%7D"}}</script>',
            ],
          }),
        },
      });
      await assets.load('lazy-card');

      assert.equal(appended.length, 1);
      assert.equal(appended[0].type, 'importmap');
      assert.equal(appended[0].nonce, 'abc123');
      assert.equal(
        appended[0].textContent,
        '{"imports":{"lazy-card":"data:text/css,body%7B%7D"}}',
      );
      assert.deepEqual(getTemplate('lazy-card'), template);
    } finally {
      restoreGlobal('window', previousWindow);
      restoreGlobal('document', previousDocument);
    }
  });

  test('manifest load registers template functions from the asset module', async () => {
    const previousWindow = setGlobal('window', { __webui: {} });
    const previousDocument = setGlobal('document', {
      baseURI: 'https://example.test/app/',
      getElementById() {
        return null;
      },
      querySelector() {
        return null;
      },
    });

    try {
      const assets = defineComponentAssets({
        'fn-card': {
          asset: assetModule(`{
            type: 'webui-component-asset',
            version: 1,
            components: ['fn-card'],
            templates: { 'fn-card': { h: '<p>Fn</p>' } },
            templateFunctions: { 'fn-card': [function(v,s){return !!v('ready',s);}] }
          }`),
        },
      });

      await assets.load('fn-card');

      const fns = window.__webui?.templateFns?.['fn-card'];
      assert.equal(typeof fns?.[0], 'function');
      assert.equal(getTemplate('fn-card')?.h, '<p>Fn</p>');
    } finally {
      restoreGlobal('window', previousWindow);
      restoreGlobal('document', previousDocument);
    }
  });

  test('manifest preload reuses in-flight work and starts module plus data', async () => {
    const previousWindow = setGlobal('window', { __webui: {} });
    const previousDocument = setGlobal('document', {
      baseURI: 'https://example.test/app/',
      createElement() {
        return { type: '', nonce: '', textContent: '' };
      },
      head: {
        appendChild(script: ScriptMock) {
          return script;
        },
      },
      getElementById() {
        return null;
      },
      querySelector() {
        return null;
      },
    });
    let moduleCount = 0;
    let dataCount = 0;

    try {
      const assets = defineComponentAssets({
        'cached-card': {
          asset: assetObjectModule({
            ...componentAsset({ 'cached-card': { h: '<p>Cached</p>' } }),
            templateStyles: [
              '<script type="importmap">{"imports":{"cached-card":"data:text/css,body%7B%7D"}}</script>',
            ],
          }),
          module: async () => {
            moduleCount += 1;
          },
          data: async () => {
            dataCount += 1;
            return { title: 'Cached data' };
          },
        },
      });

      const first = assets.preload<{ title: string }>('cached-card');
      const second = assets.preload<{ title: string }>('cached-card');
      assert.equal(first, second);
      await assets.load('cached-card');
      const data = await assets.data<{ title: string }>('cached-card');

      assert.equal(moduleCount, 1);
      assert.equal(dataCount, 1);
      assert.deepEqual(data, { title: 'Cached data' });
      assert.equal(getTemplate('cached-card')?.h, '<p>Cached</p>');
    } finally {
      restoreGlobal('window', previousWindow);
      restoreGlobal('document', previousDocument);
    }
  });

  test('manifest create applies data asynchronously by default', async () => {
    let applied: Record<string, unknown> | undefined;
    let resolveData!: (state: Record<string, unknown>) => void;
    const previousWindow = setGlobal('window', { __webui: {} });
    const previousDocument = setGlobal('document', {
      baseURI: 'https://example.test/app/',
      createElement(tag: string) {
        if (tag === 'state-card') {
          return {
            setState(state: Record<string, unknown>) {
              applied = state;
            },
          };
        }
        return { type: '', nonce: '', textContent: '' };
      },
      head: {
        appendChild(script: ScriptMock) {
          return script;
        },
      },
      getElementById() {
        return null;
      },
      querySelector() {
        return null;
      },
    });

    try {
      const assets = defineComponentAssets({
        'state-card': {
          asset: assetObjectModule(componentAsset({ 'state-card': { h: '<p>State</p>' } })),
          data: () => new Promise(resolve => {
            resolveData = resolve;
          }),
        },
      });

      const element = await assets.create('state-card');

      assert.ok(element);
      assert.equal(applied, undefined);
      resolveData({ title: 'Loaded state' });
      await Promise.resolve();
      assert.deepEqual(applied, { title: 'Loaded state' });
    } finally {
      restoreGlobal('window', previousWindow);
      restoreGlobal('document', previousDocument);
    }
  });

  test('manifest create can wait for data before returning', async () => {
    let applied: Record<string, unknown> | undefined;
    const previousWindow = setGlobal('window', { __webui: {} });
    const previousDocument = setGlobal('document', {
      baseURI: 'https://example.test/app/',
      createElement(tag: string) {
        if (tag === 'blocking-card') {
          return {
            setState(state: Record<string, unknown>) {
              applied = state;
            },
          };
        }
        return { type: '', nonce: '', textContent: '' };
      },
      head: {
        appendChild(script: ScriptMock) {
          return script;
        },
      },
      getElementById() {
        return null;
      },
      querySelector() {
        return null;
      },
    });

    try {
      const assets = defineComponentAssets({
        'blocking-card': {
          asset: assetObjectModule(componentAsset({ 'blocking-card': { h: '<p>State</p>' } })),
          data: async () => ({ title: 'Blocking state' }),
        },
      });

      const element = await assets.create('blocking-card', { awaitData: true });

      assert.ok(element);
      assert.deepEqual(applied, { title: 'Blocking state' });
    } finally {
      restoreGlobal('window', previousWindow);
      restoreGlobal('document', previousDocument);
    }
  });

  test('manifest create data timeout returns element and applies data later', async () => {
    let applied: Record<string, unknown> | undefined;
    let resolveData!: (state: Record<string, unknown>) => void;
    const previousWindow = setGlobal('window', { __webui: {} });
    const previousDocument = setGlobal('document', {
      baseURI: 'https://example.test/app/',
      createElement(tag: string) {
        if (tag === 'timeout-card') {
          return {
            setState(state: Record<string, unknown>) {
              applied = state;
            },
          };
        }
        return { type: '', nonce: '', textContent: '' };
      },
      head: {
        appendChild(script: ScriptMock) {
          return script;
        },
      },
      getElementById() {
        return null;
      },
      querySelector() {
        return null;
      },
    });

    try {
      const assets = defineComponentAssets({
        'timeout-card': {
          asset: assetObjectModule(componentAsset({ 'timeout-card': { h: '<p>State</p>' } })),
          data: () => new Promise(resolve => {
            resolveData = resolve;
          }),
        },
      });

      const element = await assets.create('timeout-card', {
        awaitData: true,
        dataTimeoutMs: 0,
      });

      assert.ok(element);
      assert.equal(applied, undefined);
      resolveData({ title: 'Late state' });
      await Promise.resolve();
      assert.deepEqual(applied, { title: 'Late state' });
    } finally {
      restoreGlobal('window', previousWindow);
      restoreGlobal('document', previousDocument);
    }
  });

  test('manifest load skips import when root template is already registered', async () => {
    const previousWindow = setGlobal('window', {
      __webui: {
        styles: ['already-loaded'],
        templates: { 'already-loaded': { h: '<p>Already loaded</p>' } },
      },
    });
    const previousDocument = setGlobal('document', {
      baseURI: 'https://example.test/app/',
      getElementById() {
        return null;
      },
    });

    try {
      const assets = defineComponentAssets({
        'already-loaded': {
          asset: 'data:text/javascript,throw%20new%20Error(%22import%20should%20not%20run%22)',
        },
      });
      await assets.load('already-loaded');

      assert.equal(getTemplate('already-loaded')?.h, '<p>Already loaded</p>');
    } finally {
      restoreGlobal('window', previousWindow);
      restoreGlobal('document', previousDocument);
    }
  });
});
