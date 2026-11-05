// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { after, test } from 'node:test';

const restore: [string, PropertyDescriptor | undefined][] = [];

function setGlobal(name: string, value: unknown): void {
  restore.push([name, Object.getOwnPropertyDescriptor(globalThis, name)]);
  Object.defineProperty(globalThis, name, { configurable: true, writable: true, value });
}

after(() => {
  for (const [name, descriptor] of restore.reverse()) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor);
    else Reflect.deleteProperty(globalThis, name);
  }
});

const runtime: NonNullable<Window['__webui']> = {};
const noMeta = (): null => null;
const page = { baseURI: 'https://site.example/news/today/', querySelector: noMeta };
setGlobal('window', { __webui: runtime });
setGlobal('document', page);

const { resolveAsset } = await import('./asset-base.js');

test('compiler paths resolve against the document base without a configured asset base', () => {
  assert.equal(resolveAsset('_webui/packs/x.json'), 'https://site.example/news/today/_webui/packs/x.json');
  assert.equal(resolveAsset('https://other.example/a.css'), 'https://other.example/a.css');
});

test('an absolute asset base known only at runtime wins over the document base', () => {
  runtime.assetBase = 'https://cdn.example/assets/v42/';
  assert.equal(resolveAsset('_webui/packs/x.json'), 'https://cdn.example/assets/v42/_webui/packs/x.json');
  assert.equal(resolveAsset('app-card.css'), 'https://cdn.example/assets/v42/app-card.css');
});

test('a relative asset base resolves against the document base first', () => {
  runtime.assetBase = '/static/';
  assert.equal(resolveAsset('app-card.css'), 'https://site.example/static/app-card.css');
  runtime.assetBase = 'assets/';
  assert.equal(resolveAsset('app-card.css'), 'https://site.example/news/today/assets/app-card.css');
});

test('the cached base follows changes to the configured value and the document base', () => {
  runtime.assetBase = 'assets/';
  assert.equal(resolveAsset('a.css'), 'https://site.example/news/today/assets/a.css');
  page.baseURI = 'https://site.example/sports/';
  assert.equal(resolveAsset('a.css'), 'https://site.example/sports/assets/a.css');
  delete runtime.assetBase;
  assert.equal(resolveAsset('a.css'), 'https://site.example/sports/a.css');
});

test('an owner document reads the asset base of its own window', () => {
  const frame = {
    baseURI: 'https://frame.example/',
    defaultView: { __webui: { assetBase: 'https://frame-cdn.example/' } },
    querySelector: noMeta,
  } as unknown as Document;
  assert.equal(resolveAsset('a.css', frame), 'https://frame-cdn.example/a.css');
  const inert = { baseURI: 'https://inert.example/', defaultView: null, querySelector: noMeta } as unknown as Document;
  assert.equal(resolveAsset('a.css', inert), 'https://inert.example/a.css');
});

/** A document whose head publishes an asset base meta, counting its lookups. */
function publishing(baseURI: string, content: string): Document & { queries: number } {
  const owner = {
    baseURI,
    defaultView: { __webui: {} },
    queries: 0,
    querySelector(selector: string) {
      owner.queries++;
      return selector === 'meta[name="webui-asset-base"]' ? { content } : null;
    },
  };
  return owner as unknown as Document & { queries: number };
}

test('the asset base meta applies before the runtime publishes a base, read once per document', () => {
  const early = publishing('https://site.example/news/', 'https://cdn.example/early/');
  assert.equal(resolveAsset('_webui/packs/x.json', early), 'https://cdn.example/early/_webui/packs/x.json');
  assert.equal(resolveAsset('a.css', early), 'https://cdn.example/early/a.css');
  assert.equal(early.queries, 1, 'no DOM query after the first');

  const runtimeGlobal = (early.defaultView as unknown as { __webui: { assetBase?: string } }).__webui;
  runtimeGlobal.assetBase = 'https://cdn.example/late/';
  assert.equal(resolveAsset('a.css', early), 'https://cdn.example/late/a.css', 'the runtime base wins');
  delete runtimeGlobal.assetBase;
  assert.equal(resolveAsset('a.css', early), 'https://cdn.example/early/a.css');
  assert.equal(early.queries, 1);

  const relative = publishing('https://site.example/news/today/', '/static/');
  assert.equal(resolveAsset('a.css', relative), 'https://site.example/static/a.css', 'a relative meta resolves against the document');
  assert.equal(resolveAsset('a.css', early), 'https://cdn.example/early/a.css', 'each document keeps its own meta');
  assert.deepEqual([early.queries, relative.queries], [1, 1]);
});

test('paths that cannot resolve and documentless runtimes keep the input', () => {
  runtime.assetBase = 'https://[invalid/';
  assert.equal(resolveAsset('a.css'), 'a.css');
  runtime.assetBase = 'https://cdn.example/';
  assert.equal(resolveAsset('a.css'), 'https://cdn.example/a.css', 'a failed base is not cached');
  setGlobal('document', undefined);
  assert.equal(resolveAsset('a.css'), 'a.css');
});
