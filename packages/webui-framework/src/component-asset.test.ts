// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';

import { takeGeneratedComponentAssetStyles } from './component-asset/generated-manifest.js';
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
  const components = Object.keys(templates);
  return {
    type: 'webui-component-asset',
    version: 2,
    kind: 'root',
    root: components[0],
    components,
    requiredComponents: components,
    externalComponents: [],
    imports: [],
    templateStyles: [],
    templates,
  };
}

describe('component asset helpers', () => {
  test('generated style metadata is a no-op without browser globals', () => {
    const previousWindow = setGlobal('window', undefined);
    const previousDocument = setGlobal('document', undefined);

    try {
      assert.equal(takeGeneratedComponentAssetStyles('server-card'), undefined);
    } finally {
      restoreGlobal('window', previousWindow);
      restoreGlobal('document', previousDocument);
    }
  });

  test('loads compiler-generated style metadata beside the authored asset', async () => {
    const template: TemplateMeta = { h: '<p>Generated</p>' };
    const asset = assetObjectModule(componentAsset({ 'generated-card': template }));
    let removed = false;
    const previousWindow = setGlobal('window', { __webui: {} });
    const previousDocument = setGlobal('document', {
      baseURI: 'https://example.test/app/',
      getElementById(id: string) {
        if (id === 'webui-data') return null;
        assert.equal(id, 'webui-component-assets');
        return {
          textContent: JSON.stringify({
            'generated-card': ['/generated-card.css'],
          }),
          remove() {
            removed = true;
          },
        };
      },
      querySelector() {
        return null;
      },
    });

    try {
      const assets = defineComponentAssets({
        'generated-card': { asset },
      });
      await assets.preload('generated-card').asset;

      assert.equal(removed, true);
      assert.deepEqual(window.__webui?.componentAssetStyles, {});
      assert.deepEqual(getTemplate('generated-card'), template);
    } finally {
      restoreGlobal('window', previousWindow);
      restoreGlobal('document', previousDocument);
    }
  });

  test('accepts a bundler-owned asset importer and invokes it once', async () => {
    const template: TemplateMeta = { h: '<p>Bundled</p>' };
    let imports = 0;
    const previousWindow = setGlobal('window', { __webui: {} });
    const previousDocument = setGlobal('document', {
      getElementById() {
        return null;
      },
      querySelector() {
        return null;
      },
    });

    try {
      const assets = defineComponentAssets({
        'bundled-card': {
          asset: async () => {
            imports += 1;
            return {
              default: componentAsset({ 'bundled-card': template }),
            };
          },
        },
      });
      const first = assets.preload('bundled-card');
      const second = assets.preload('bundled-card');
      await Promise.all([first.asset, second.asset]);

      assert.equal(first, second);
      assert.equal(imports, 1);
      assert.deepEqual(getTemplate('bundled-card'), template);
    } finally {
      restoreGlobal('window', previousWindow);
      restoreGlobal('document', previousDocument);
    }
  });

  test('manifest preload registers templates and injects nonce importmaps', async () => {
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
      await assets.preload('lazy-card').asset;

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

  test('manifest preload rejects non-importmap template styles', async () => {
    const previousWindow = setGlobal('window', { __webui: {} });
    const previousDocument = setGlobal('document', {
      baseURI: 'https://example.test/app/',
      createElement() {
        return { type: '', nonce: '', textContent: '' };
      },
      getElementById() {
        return null;
      },
      head: {
        appendChild(script: ScriptMock) {
          return script;
        },
      },
      querySelector() {
        return null;
      },
    });

    try {
      const assets = defineComponentAssets({
        'invalid-style-card': {
          asset: assetObjectModule({
            ...componentAsset({
              'invalid-style-card': { h: '<p>Invalid style</p>' },
            }),
            templateStyles: [
              '<script type="application/json">{"imports":{"invalid-style-card":"data:text/css,body%7B%7D"}}</script>',
            ],
          }),
        },
      });

      await assert.rejects(
        assets.preload('invalid-style-card').asset,
        /must be a <script type="importmap"> tag/,
      );
      assert.equal(window.__webui?.templates, undefined);
    } finally {
      restoreGlobal('window', previousWindow);
      restoreGlobal('document', previousDocument);
    }
  });

  test('manifest preload registers template functions from the asset module', async () => {
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
            version: 2,
            kind: 'root',
            root: 'fn-card',
            components: ['fn-card'],
            requiredComponents: ['fn-card'],
            externalComponents: [],
            imports: [],
            templateStyles: [],
            templates: { 'fn-card': { h: '<p>Fn</p>' } },
            templateFunctions: { 'fn-card': [function(v,s){return !!v('ready',s);}] }
          }`),
        },
      });

      await assets.preload('fn-card').asset;

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
      await first.asset;
      if (first.module) await first.module;
      const data = first.data ? await first.data : undefined;

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

  test('manifest preload imports the root graph when its root template is already registered', async () => {
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
      const asset = JSON.stringify(componentAsset({
        'already-loaded': { h: '<p>Already loaded</p>' },
      }));
      const assets = defineComponentAssets({
        'already-loaded': {
          asset: assetModule(
            `(globalThis.__componentAssetImportCount = (globalThis.__componentAssetImportCount ?? 0) + 1, ${asset})`,
          ),
        },
      });
      await assets.preload('already-loaded').asset;

      assert.equal(getTemplate('already-loaded')?.h, '<p>Already loaded</p>');
      assert.equal(
        (globalThis as typeof globalThis & { __componentAssetImportCount?: number })
          .__componentAssetImportCount,
        1,
      );
    } finally {
      Reflect.deleteProperty(globalThis, '__componentAssetImportCount');
      restoreGlobal('window', previousWindow);
      restoreGlobal('document', previousDocument);
    }
  });

  test('manifest preload rejects version 1 assets', async () => {
    const previousWindow = setGlobal('window', { __webui: {} });
    const previousDocument = setGlobal('document', {
      baseURI: 'https://example.test/app/',
    });

    try {
      const assets = defineComponentAssets({
        'legacy-card': {
          asset: assetObjectModule({
            type: 'webui-component-asset',
            version: 1,
            components: ['legacy-card'],
            templates: { 'legacy-card': { h: '<p>Legacy</p>' } },
          }),
        },
      });

      await assert.rejects(
        assets.preload('legacy-card').asset,
        /Unsupported component asset version: 1/,
      );
    } finally {
      restoreGlobal('window', previousWindow);
      restoreGlobal('document', previousDocument);
    }
  });

  test('manifest preload rejects an empty root graph', async () => {
    const previousWindow = setGlobal('window', { __webui: {} });
    const previousDocument = setGlobal('document', {
      baseURI: 'https://example.test/app/',
    });

    try {
      const assets = defineComponentAssets({
        'empty-card': {
          asset: assetObjectModule({
            type: 'webui-component-asset',
            version: 2,
            kind: 'root',
            root: 'empty-card',
            components: [],
            requiredComponents: [],
            externalComponents: [],
            imports: [],
            templateStyles: [],
            templates: {},
          }),
        },
      });

      await assert.rejects(
        assets.preload('empty-card').asset,
        /root <empty-card> must include itself in requiredComponents/,
      );
      assert.equal(getTemplate('empty-card'), undefined);
    } finally {
      restoreGlobal('window', previousWindow);
      restoreGlobal('document', previousDocument);
    }
  });

  test('manifest preload rejects undeclared template payloads before registration', async () => {
    const previousWindow = setGlobal('window', { __webui: {} });
    const previousDocument = setGlobal('document', {
      baseURI: 'https://example.test/app/',
    });

    try {
      const assets = defineComponentAssets({
        'declared-card': {
          asset: assetObjectModule({
            type: 'webui-component-asset',
            version: 2,
            kind: 'root',
            root: 'declared-card',
            components: ['declared-card'],
            requiredComponents: ['declared-card'],
            externalComponents: [],
            imports: [],
            templateStyles: [],
            templates: { 'undeclared-card': { h: '<p>Wrong</p>' } },
          }),
        },
      });

      await assert.rejects(
        assets.preload('declared-card').asset,
        /templates contain undeclared payload <undeclared-card>/,
      );
      assert.equal(getTemplate('undeclared-card'), undefined);
    } finally {
      restoreGlobal('window', previousWindow);
      restoreGlobal('document', previousDocument);
    }
  });

  test('invalid condition indexes leave the whole template batch unregistered', async () => {
    const previousWindow = setGlobal('window', { __webui: {} });
    const previousDocument = setGlobal('document', {
      baseURI: 'https://example.test/app/',
    });

    try {
      const assets = defineComponentAssets({
        'condition-card': {
          asset: assetModule(`{
            type: 'webui-component-asset',
            version: 2,
            kind: 'root',
            root: 'condition-card',
            components: ['valid-child', 'condition-card'],
            requiredComponents: ['valid-child', 'condition-card'],
            externalComponents: [],
            imports: [],
            templateStyles: [],
            templates: {
              'valid-child': { h: '<p>Valid</p>' },
              'condition-card': {
                h: '<valid-child></valid-child>',
                c: [[[1, ['ready']], 0, [[], 0]]]
              }
            },
            templateFunctions: {
              'condition-card': [function(v,s){return !!v('ready',s);}]
            }
          }`),
        },
      });

      await assert.rejects(
        assets.preload('condition-card').asset,
        /Missing condition closure 1 for <condition-card>/,
      );
      assert.equal(window.__webui?.templates, undefined);
      assert.equal(window.__webui?.templateFns, undefined);
    } finally {
      restoreGlobal('window', previousWindow);
      restoreGlobal('document', previousDocument);
    }
  });

  test('asset conditions cannot reuse stale global closure arrays', async () => {
    const previousWindow = setGlobal('window', {
      __webui: {
        templateFns: {
          'stale-condition-card': [() => true],
        },
      },
    });
    const previousDocument = setGlobal('document', {
      baseURI: 'https://example.test/app/',
    });

    try {
      const assets = defineComponentAssets({
        'stale-condition-card': {
          asset: assetObjectModule({
            type: 'webui-component-asset',
            version: 2,
            kind: 'root',
            root: 'stale-condition-card',
            components: ['stale-condition-card'],
            requiredComponents: ['stale-condition-card'],
            externalComponents: [],
            imports: [],
            templateStyles: [],
            templates: {
              'stale-condition-card': {
                h: '<!--wc:0--><!--/wc-->',
                b: [{ h: '<p>Ready</p>' }],
                c: [[[0, ['ready']], 0, [[], 0]]],
              },
            },
          }),
        },
      });

      await assert.rejects(
        assets.preload('stale-condition-card').asset,
        /Missing condition closure 0 for <stale-condition-card>/,
      );
      assert.equal(window.__webui?.templates, undefined);
    } finally {
      restoreGlobal('window', previousWindow);
      restoreGlobal('document', previousDocument);
    }
  });

  test('concurrent manifest tags validate a shared asset URL independently', async () => {
    const previousWindow = setGlobal('window', { __webui: {} });
    const previousDocument = setGlobal('document', {
      baseURI: 'https://example.test/app/',
      querySelector() {
        return null;
      },
    });
    const sharedAssetUrl = assetObjectModule(componentAsset({
      'first-url-panel': { h: '<p>First</p>' },
    }));

    try {
      const first = defineComponentAssets({
        'first-url-panel': { asset: sharedAssetUrl },
      });
      const second = defineComponentAssets({
        'second-url-panel': { asset: sharedAssetUrl },
      });
      const results = await Promise.allSettled([
        first.preload('first-url-panel').asset,
        second.preload('second-url-panel').asset,
      ]);

      assert.equal(results[0].status, 'fulfilled');
      assert.equal(results[1].status, 'rejected');
      if (results[1].status === 'rejected') {
        assert.match(
          String(results[1].reason),
          /expected <second-url-panel>.*exports <first-url-panel>/,
        );
      }
      assert.equal(getTemplate('second-url-panel'), undefined);
    } finally {
      restoreGlobal('window', previousWindow);
      restoreGlobal('document', previousDocument);
    }
  });

  test('concurrent roots import and register one shared chunk once', async () => {
    const previousWindow = setGlobal('window', { __webui: {} });
    const previousDocument = setGlobal('document', {
      baseURI: 'https://example.test/app/',
      querySelector() {
        return null;
      },
    });
    const chunkUrl = assetObjectModule({
      type: 'webui-component-asset',
      version: 2,
      kind: 'chunk',
      components: ['shared-detail'],
      requiredComponents: ['shared-detail'],
      externalComponents: [],
      imports: [],
      templateStyles: [],
      templates: { 'shared-detail': { h: '<p>Shared</p>' } },
    });
    const chunkUrlSource = JSON.stringify(chunkUrl);
    const rootModule = (root: string) => assetModule(`{
      type: 'webui-component-asset',
      version: 2,
      kind: 'root',
      root: '${root}',
      components: ['${root}'],
      requiredComponents: ['${root}', 'shared-detail'],
      externalComponents: [],
      imports: [{
        components: ['shared-detail'],
        href: ${chunkUrlSource},
        load: () => {
          globalThis.__componentAssetChunkLoads =
            (globalThis.__componentAssetChunkLoads ?? 0) + 1;
          return import(${chunkUrlSource});
        }
      }],
      templateStyles: [],
      templates: { '${root}': { h: '<shared-detail></shared-detail>' } }
    }`);

    try {
      const first = defineComponentAssets({
        'first-panel': { asset: rootModule('first-panel') },
      });
      const second = defineComponentAssets({
        'second-panel': { asset: rootModule('second-panel') },
      });

      await Promise.all([
        first.preload('first-panel').asset,
        second.preload('second-panel').asset,
      ]);

      assert.equal(
        (globalThis as typeof globalThis & { __componentAssetChunkLoads?: number })
          .__componentAssetChunkLoads,
        1,
      );
      assert.equal(getTemplate('shared-detail')?.h, '<p>Shared</p>');
      assert.equal(getTemplate('first-panel')?.h, '<shared-detail></shared-detail>');
      assert.equal(getTemplate('second-panel')?.h, '<shared-detail></shared-detail>');
    } finally {
      Reflect.deleteProperty(globalThis, '__componentAssetChunkLoads');
      restoreGlobal('window', previousWindow);
      restoreGlobal('document', previousDocument);
    }
  });

  test('missing entry prerequisites fail before root registration', async () => {
    const previousWindow = setGlobal('window', { __webui: {} });
    const previousDocument = setGlobal('document', {
      baseURI: 'https://example.test/app/',
      querySelector() {
        return null;
      },
    });
    const root = {
      ...componentAsset({ 'external-root': { h: '<entry-owned></entry-owned>' } }),
      requiredComponents: ['entry-owned', 'external-root'],
      externalComponents: ['entry-owned'],
    };

    try {
      const assets = defineComponentAssets({
        'external-root': { asset: assetObjectModule(root) },
      });

      await assert.rejects(
        assets.preload('external-root').asset,
        /requires entry template <entry-owned>/,
      );
      assert.equal(getTemplate('external-root'), undefined);
    } finally {
      restoreGlobal('window', previousWindow);
      restoreGlobal('document', previousDocument);
    }
  });
});
