// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';
import {
  installComponentStyles,
  registerComponentStyles,
  setCssModuleLoaderForTests,
  type ComponentStyles,
} from './styles.js';

class FakeElement {
  type = '';
  rel = '';
  href = '';
  nonce = '';
  textContent = '';
  private readonly attributes = new Map<string, string>();

  setAttribute(name: string, value: string): void {
    this.attributes.set(name, value);
  }

  getAttribute(name: string): string | null {
    return this.attributes.get(name) ?? null;
  }

  remove(): void {}
}

class FakeParent {
  readonly children: FakeElement[] = [];

  querySelectorAll(): FakeElement[] {
    return this.children.filter(child =>
      child.getAttribute('data-webui-resource') !== null
    );
  }

  appendChild(child: FakeElement): FakeElement {
    this.children.push(child);
    return child;
  }

  insertBefore(child: FakeElement, before: FakeElement | null): FakeElement {
    const index = before ? this.children.indexOf(before) : -1;
    if (index < 0) this.children.push(child);
    else this.children.splice(index, 0, child);
    return child;
  }
}

function fakeDocument(globalNonce = '', metaNonce = ''): Document {
  const head = new FakeParent();
  return {
    nodeType: 9,
    head,
    createElement: () => new FakeElement(),
    defaultView: globalNonce ? { __webui: { nonce: globalNonce } } : null,
    querySelector: () => metaNonce ? { content: metaNonce } : null,
  } as unknown as Document;
}

function fakeShadow(ownerDocument: Document): ShadowRoot {
  const root = new FakeParent() as FakeParent & {
    nodeType: number;
    ownerDocument: Document;
    adoptedStyleSheets: CSSStyleSheet[];
  };
  root.nodeType = 11;
  root.ownerDocument = ownerDocument;
  root.adoptedStyleSheets = [];
  return root as unknown as ShadowRoot;
}

function styles(closures: Record<string, string[]>): ComponentStyles {
  return {
    version: 1,
    strategy: 'style',
    resources: {
      first: { kind: 'style', css: '.first{}' },
      second: { kind: 'style', css: '.second{}' },
    },
    closures,
  };
}

describe('component style resources', () => {
  test('rejects registration without componentStyles', () => {
    assert.throws(
      () => registerComponentStyles(undefined, fakeDocument()),
      /componentStyles is required/,
    );
  });

  test('installs once per Document in closure order', async () => {
    const document = fakeDocument();
    registerComponentStyles(styles({ root: ['second', 'first'] }), document);

    await installComponentStyles('root', document);
    await installComponentStyles('root', document);

    const children = (document.head as unknown as FakeParent).children;
    assert.deepEqual(
      children.map(child => child.getAttribute('data-webui-resource')),
      ['second', 'first'],
    );
  });

  test('installs once in each ShadowRoot without a global proxy', async () => {
    const document = fakeDocument();
    const firstRoot = fakeShadow(document);
    const secondRoot = fakeShadow(document);
    registerComponentStyles(styles({ root: ['first'] }), document);

    await installComponentStyles('root', firstRoot);
    await installComponentStyles('root', firstRoot);
    await installComponentStyles('root', secondRoot);

    assert.equal((firstRoot as unknown as FakeParent).children.length, 1);
    assert.equal((secondRoot as unknown as FakeParent).children.length, 1);
  });

  test('applies the owning Document nonce to Document and ShadowRoot styles', async () => {
    const document = fakeDocument('document-nonce');
    const root = fakeShadow(document);
    registerComponentStyles(styles({ root: ['first'] }), document);

    await installComponentStyles('root', document);
    await installComponentStyles('root', root);

    assert.equal(
      (document.head as unknown as FakeParent).children[0].nonce,
      'document-nonce',
    );
    assert.equal(
      (root as unknown as FakeParent).children[0].nonce,
      'document-nonce',
    );

    const metaDocument = fakeDocument('', 'meta-nonce');
    const metaRoot = fakeShadow(metaDocument);
    registerComponentStyles(styles({ root: ['first'] }), metaDocument);
    await installComponentStyles('root', metaRoot);
    assert.equal(
      (metaRoot as unknown as FakeParent).children[0].nonce,
      'meta-nonce',
    );
  });

  test('claims only an exact SSR resource marker', async () => {
    const document = fakeDocument();
    const root = fakeShadow(document);
    const nearMatch = new FakeElement();
    nearMatch.setAttribute('data-webui-resource', 'first-child');
    const exact = new FakeElement();
    exact.setAttribute('data-webui-resource', 'first');
    (root as unknown as FakeParent).appendChild(nearMatch);
    (root as unknown as FakeParent).appendChild(exact);
    registerComponentStyles(styles({ root: ['first'] }), document);

    await installComponentStyles('root', root);

    assert.equal((root as unknown as FakeParent).children.length, 2);
  });

  test('rejects conflicting definitions before publishing their closure', async () => {
    const document = fakeDocument();
    registerComponentStyles(styles({ root: ['first'] }), document);

    assert.throws(() => registerComponentStyles({
      version: 1,
      strategy: 'style',
      resources: { first: { kind: 'style', css: '.conflict{}' } },
      closures: { conflicting: ['first'] },
    }, document), /Conflicting component style resource/);

    await installComponentStyles('conflicting', document);
    assert.equal((document.head as unknown as FakeParent).children.length, 0);
  });

  test('keeps unrelated adopted sheets and clears failed Module work for retry', async () => {
    const document = fakeDocument();
    const root = fakeShadow(document);
    const unrelated = {} as CSSStyleSheet;
    const componentSheet = {} as CSSStyleSheet;
    root.adoptedStyleSheets = [unrelated];
    registerComponentStyles({
      version: 1,
      strategy: 'module',
      resources: {
        module: { kind: 'module', specifier: 'module', css: '.retry{}' },
      },
      closures: { root: ['module'] },
    }, document);
    let attempts = 0;
    setCssModuleLoaderForTests(() => {
      attempts++;
      return attempts === 1
        ? Promise.reject(new Error('not registered yet'))
        : Promise.resolve({ default: componentSheet });
    });

    await assert.rejects(installComponentStyles('root', root));
    await installComponentStyles('root', root);
    setCssModuleLoaderForTests();

    assert.equal(attempts, 2);
    assert.deepEqual(root.adoptedStyleSheets, [unrelated, componentSheet]);

    const importMaps = (document.head as unknown as FakeParent).children.filter(
      (child) => (child as unknown as { type: string }).type === 'importmap',
    );
    assert.equal(importMaps.length, 1);
  });

  test('loads and adopts Module resources in closure order', async () => {
    const document = fakeDocument();
    const root = fakeShadow(document);
    registerComponentStyles({
      version: 1,
      strategy: 'module',
      resources: {
        dependency: { kind: 'module', specifier: 'dependency', css: '.dependency{}' },
        component: { kind: 'module', specifier: 'component', css: '.component{}' },
      },
      closures: { component: ['dependency', 'component'] },
    }, document);
    const loaded: string[] = [];
    setCssModuleLoaderForTests((specifier) => {
      loaded.push(specifier);
      return Promise.resolve({ default: {} as CSSStyleSheet });
    });

    await installComponentStyles('component', root);
    setCssModuleLoaderForTests();

    assert.deepEqual(loaded, ['dependency', 'component']);
    assert.equal(root.adoptedStyleSheets.length, 2);
  });

  test('installs one import map per Module specifier per Document', async () => {
    const document = fakeDocument();
    const firstRoot = fakeShadow(document);
    const secondRoot = fakeShadow(document);
    registerComponentStyles({
      version: 1,
      strategy: 'module',
      resources: {
        first: { kind: 'module', specifier: 'shared-specifier', css: 'body{}' },
        second: { kind: 'module', specifier: 'shared-specifier', css: 'body{}' },
      },
      closures: { first: ['first'], second: ['second'] },
    }, document);
    setCssModuleLoaderForTests(() => Promise.resolve({ default: {} as CSSStyleSheet }));

    await installComponentStyles('first', firstRoot);
    await installComponentStyles('second', secondRoot);
    setCssModuleLoaderForTests();

    const importMaps = (document.head as unknown as FakeParent).children.filter(
      (child) => (child as unknown as { type: string }).type === 'importmap',
    );
    assert.equal(importMaps.length, 1);
    assert.equal(
      (importMaps[0] as unknown as { textContent: string }).textContent,
      '{"imports":{"shared-specifier":"data:text/css,body%7B%7D"}}',
    );
  });

  test('applies the owning Document nonce to the module import map', async () => {
    const document = fakeDocument('document-nonce');
    const root = fakeShadow(document);
    registerComponentStyles({
      version: 1,
      strategy: 'module',
      resources: {
        themed: { kind: 'module', specifier: 'themed-specifier', css: '.themed{}' },
      },
      closures: { root: ['themed'] },
    }, document);
    setCssModuleLoaderForTests(() => Promise.resolve({ default: {} as CSSStyleSheet }));

    await installComponentStyles('root', root);
    setCssModuleLoaderForTests();

    const importMap = (document.head as unknown as FakeParent).children.find(
      (child) => (child as unknown as { type: string }).type === 'importmap',
    );
    assert.equal((importMap as unknown as { nonce: string } | undefined)?.nonce, 'document-nonce');
  });

  test('does not redefine module specifiers already seeded by the SSR bootstrap', async () => {
    const document = fakeDocument();
    (document as unknown as { defaultView: { __webui: { styles: string[] } } }).defaultView = {
      __webui: { styles: ['seeded-specifier'] },
    };
    const root = fakeShadow(document);
    registerComponentStyles({
      version: 1,
      strategy: 'module',
      resources: {
        seeded: { kind: 'module', specifier: 'seeded-specifier', css: 'body{}' },
      },
      closures: { root: ['seeded'] },
    }, document);
    setCssModuleLoaderForTests(() => Promise.resolve({ default: {} as CSSStyleSheet }));

    await installComponentStyles('root', root);
    setCssModuleLoaderForTests();

    const importMaps = (document.head as unknown as FakeParent).children.filter(
      (child) => (child as unknown as { type: string }).type === 'importmap',
    );
    assert.equal(importMaps.length, 0);
  });
});
