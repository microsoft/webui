// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';

import { getTemplate, type TemplateMeta } from './template.js';
import { defineComponentAssets } from './component-asset.js';

describe('component asset helpers', () => {
  test('manifest load registers templates and injects nonce importmaps', async () => {
    const previousWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
    const previousDocument = Object.getOwnPropertyDescriptor(globalThis, 'document');
    const previousFetch = Object.getOwnPropertyDescriptor(globalThis, 'fetch');
    const appended: Array<{ type: string; nonce: string; textContent: string }> = [];
    const template: TemplateMeta = { h: '<p>Lazy</p>' };

    try {
      Object.defineProperty(globalThis, 'window', {
        value: { __webui: { nonce: 'abc123' } },
        configurable: true,
        writable: true,
      });
      Object.defineProperty(globalThis, 'document', {
        value: {
          baseURI: 'https://example.test/app/',
          createElement(tag: string) {
            assert.equal(tag, 'script');
            return { type: '', nonce: '', textContent: '' };
          },
          head: {
            appendChild(script: { type: string; nonce: string; textContent: string }) {
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
        },
        configurable: true,
        writable: true,
      });
      Object.defineProperty(globalThis, 'fetch', {
        value: async () => ({
          ok: true,
          async json() {
            return {
              type: 'webui-component-asset',
              version: 1,
              components: ['lazy-card'],
              templateStyles: [
                '<script type="importmap">{"imports":{"lazy-card":"data:text/css,body%7B%7D"}}</script>',
              ],
              templates: { 'lazy-card': template },
            };
          },
        }),
        configurable: true,
        writable: true,
      });

      const assets = defineComponentAssets({
        'lazy-card': { asset: '/lazy-card.webui.json' },
      });
      await assets.load('lazy-card');

      assert.equal(appended.length, 1);
      assert.equal(appended[0].type, 'importmap');
      assert.equal(appended[0].nonce, 'abc123');
      assert.equal(
        appended[0].textContent,
        '{"imports":{"lazy-card":"data:text/css,body%7B%7D"}}',
      );
      assert.equal(getTemplate('lazy-card'), template);
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
      if (previousFetch) {
        Object.defineProperty(globalThis, 'fetch', previousFetch);
      } else {
        Reflect.deleteProperty(globalThis, 'fetch');
      }
    }
  });

  test('manifest preload reuses in-flight work and starts module plus data', async () => {
    const previousWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
    const previousDocument = Object.getOwnPropertyDescriptor(globalThis, 'document');
    const previousFetch = Object.getOwnPropertyDescriptor(globalThis, 'fetch');
    let fetchCount = 0;
    let appended = 0;
    let moduleCount = 0;
    let dataCount = 0;

    try {
      Object.defineProperty(globalThis, 'window', {
        value: { __webui: {} },
        configurable: true,
        writable: true,
      });
      Object.defineProperty(globalThis, 'document', {
        value: {
          baseURI: 'https://example.test/app/',
          createElement() {
            return { type: '', nonce: '', textContent: '' };
          },
          head: {
            appendChild(script: { type: string; nonce: string; textContent: string }) {
              appended += 1;
              return script;
            },
          },
          getElementById() {
            return null;
          },
          querySelector() {
            return null;
          },
        },
        configurable: true,
        writable: true,
      });
      Object.defineProperty(globalThis, 'fetch', {
        value: async () => {
          fetchCount += 1;
          return {
            ok: true,
            async json() {
              return {
                type: 'webui-component-asset',
                version: 1,
                components: ['cached-card'],
                templateStyles: [
                  '<script type="importmap">{"imports":{"cached-card":"data:text/css,body%7B%7D"}}</script>',
                ],
                templates: { 'cached-card': { h: '<p>Cached</p>' } },
              };
            },
          };
        },
        configurable: true,
        writable: true,
      });

      const assets = defineComponentAssets({
        'cached-card': {
          asset: './cached-card.webui.json',
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

      assert.equal(fetchCount, 1);
      assert.equal(appended, 1);
      assert.equal(moduleCount, 1);
      assert.equal(dataCount, 1);
      assert.deepEqual(data, { title: 'Cached data' });
      assert.equal(getTemplate('cached-card')?.h, '<p>Cached</p>');
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
      if (previousFetch) {
        Object.defineProperty(globalThis, 'fetch', previousFetch);
      } else {
        Reflect.deleteProperty(globalThis, 'fetch');
      }
    }
  });

  test('manifest create applies data asynchronously by default', async () => {
    const previousWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
    const previousDocument = Object.getOwnPropertyDescriptor(globalThis, 'document');
    const previousFetch = Object.getOwnPropertyDescriptor(globalThis, 'fetch');
    let applied: Record<string, unknown> | undefined;
    let resolveData!: (state: Record<string, unknown>) => void;

    try {
      Object.defineProperty(globalThis, 'window', {
        value: { __webui: {} },
        configurable: true,
        writable: true,
      });
      Object.defineProperty(globalThis, 'document', {
        value: {
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
            appendChild(script: { type: string; nonce: string; textContent: string }) {
              return script;
            },
          },
          getElementById() {
            return null;
          },
          querySelector() {
            return null;
          },
        },
        configurable: true,
        writable: true,
      });
      Object.defineProperty(globalThis, 'fetch', {
        value: async () => ({
          ok: true,
          async json() {
            return {
              type: 'webui-component-asset',
              version: 1,
              components: ['state-card'],
              templates: { 'state-card': { h: '<p>State</p>' } },
            };
          },
        }),
        configurable: true,
        writable: true,
      });

      const assets = defineComponentAssets({
        'state-card': {
          asset: './state-card.webui.json',
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
      if (previousFetch) {
        Object.defineProperty(globalThis, 'fetch', previousFetch);
      } else {
        Reflect.deleteProperty(globalThis, 'fetch');
      }
    }
  });

  test('manifest create can wait for data before returning', async () => {
    const previousWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
    const previousDocument = Object.getOwnPropertyDescriptor(globalThis, 'document');
    const previousFetch = Object.getOwnPropertyDescriptor(globalThis, 'fetch');
    let applied: Record<string, unknown> | undefined;

    try {
      Object.defineProperty(globalThis, 'window', {
        value: { __webui: {} },
        configurable: true,
        writable: true,
      });
      Object.defineProperty(globalThis, 'document', {
        value: {
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
            appendChild(script: { type: string; nonce: string; textContent: string }) {
              return script;
            },
          },
          getElementById() {
            return null;
          },
          querySelector() {
            return null;
          },
        },
        configurable: true,
        writable: true,
      });
      Object.defineProperty(globalThis, 'fetch', {
        value: async () => ({
          ok: true,
          async json() {
            return {
              type: 'webui-component-asset',
              version: 1,
              components: ['blocking-card'],
              templates: { 'blocking-card': { h: '<p>State</p>' } },
            };
          },
        }),
        configurable: true,
        writable: true,
      });

      const assets = defineComponentAssets({
        'blocking-card': {
          asset: './blocking-card.webui.json',
          data: async () => ({ title: 'Blocking state' }),
        },
      });

      const element = await assets.create('blocking-card', { awaitData: true });

      assert.ok(element);
      assert.deepEqual(applied, { title: 'Blocking state' });
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
      if (previousFetch) {
        Object.defineProperty(globalThis, 'fetch', previousFetch);
      } else {
        Reflect.deleteProperty(globalThis, 'fetch');
      }
    }
  });

  test('manifest create data timeout returns element and applies data later', async () => {
    const previousWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
    const previousDocument = Object.getOwnPropertyDescriptor(globalThis, 'document');
    const previousFetch = Object.getOwnPropertyDescriptor(globalThis, 'fetch');
    let applied: Record<string, unknown> | undefined;
    let resolveData!: (state: Record<string, unknown>) => void;

    try {
      Object.defineProperty(globalThis, 'window', {
        value: { __webui: {} },
        configurable: true,
        writable: true,
      });
      Object.defineProperty(globalThis, 'document', {
        value: {
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
            appendChild(script: { type: string; nonce: string; textContent: string }) {
              return script;
            },
          },
          getElementById() {
            return null;
          },
          querySelector() {
            return null;
          },
        },
        configurable: true,
        writable: true,
      });
      Object.defineProperty(globalThis, 'fetch', {
        value: async () => ({
          ok: true,
          async json() {
            return {
              type: 'webui-component-asset',
              version: 1,
              components: ['timeout-card'],
              templates: { 'timeout-card': { h: '<p>State</p>' } },
            };
          },
        }),
        configurable: true,
        writable: true,
      });

      const assets = defineComponentAssets({
        'timeout-card': {
          asset: './timeout-card.webui.json',
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
      if (previousFetch) {
        Object.defineProperty(globalThis, 'fetch', previousFetch);
      } else {
        Reflect.deleteProperty(globalThis, 'fetch');
      }
    }
  });

  test('manifest load skips fetch when root template is already registered', async () => {
    const previousWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
    const previousDocument = Object.getOwnPropertyDescriptor(globalThis, 'document');
    const previousFetch = Object.getOwnPropertyDescriptor(globalThis, 'fetch');
    let fetchCount = 0;

    try {
      Object.defineProperty(globalThis, 'window', {
        value: {
          __webui: {
            styles: ['already-loaded'],
            templates: { 'already-loaded': { h: '<p>Already loaded</p>' } },
          },
        },
        configurable: true,
        writable: true,
      });
      Object.defineProperty(globalThis, 'document', {
        value: {
          baseURI: 'https://example.test/app/',
          getElementById() {
            return null;
          },
        },
        configurable: true,
        writable: true,
      });
      Object.defineProperty(globalThis, 'fetch', {
        value: async () => {
          fetchCount += 1;
          throw new Error('fetch should not run');
        },
        configurable: true,
        writable: true,
      });

      const assets = defineComponentAssets({
        'already-loaded': { asset: '/app/already-loaded.webui.json' },
      });
      await assets.load('already-loaded');

      assert.equal(fetchCount, 0);
      assert.equal(getTemplate('already-loaded')?.h, '<p>Already loaded</p>');
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
      if (previousFetch) {
        Object.defineProperty(globalThis, 'fetch', previousFetch);
      } else {
        Reflect.deleteProperty(globalThis, 'fetch');
      }
    }
  });
});
