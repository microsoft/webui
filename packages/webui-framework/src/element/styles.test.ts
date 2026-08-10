// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';
import {
  claimSsrComponentStyles,
  installComponentStyles,
  registerComponentStyles,
  setCssModuleLoaderForTests,
  type ComponentStyles,
} from './styles.js';

class FakeElement {
  localName: string;
  type = '';
  rel = '';
  href = '';
  nonce = '';
  textContent = '';
  removed = false;
  parent?: FakeParent;
  private readonly attributes = new Map<string, string>();

  constructor(localName = '') {
    this.localName = localName;
  }

  setAttribute(name: string, value: string): void {
    this.attributes.set(name, value);
  }

  getAttribute(name: string): string | null {
    return this.attributes.get(name) ?? null;
  }

  remove(): void {
    this.removed = true;
    this.parent?.detach(this);
  }
}

class FakeParent {
  private readonly childElements: FakeElement[] = [];
  private readonly nestedElements: FakeElement[] = [];
  markerScans = 0;

  get children(): FakeElement[] {
    this.markerScans++;
    return this.childElements;
  }

  querySelectorAll(): FakeElement[] {
    return this.childElements.concat(this.nestedElements).filter(child =>
      child.getAttribute('data-webui-resource') !== null
    );
  }

  appendChild(child: FakeElement): FakeElement {
    child.parent?.detach(child);
    this.childElements.push(child);
    child.parent = this;
    return child;
  }

  insertBefore(child: FakeElement, before: FakeElement | null): FakeElement {
    child.parent?.detach(child);
    const index = before ? this.childElements.indexOf(before) : -1;
    if (index < 0) this.childElements.push(child);
    else this.childElements.splice(index, 0, child);
    child.parent = this;
    return child;
  }

  appendNested(child: FakeElement): FakeElement {
    child.parent?.detach(child);
    this.nestedElements.push(child);
    child.parent = this;
    return child;
  }

  detach(child: FakeElement): void {
    let index = this.childElements.indexOf(child);
    if (index >= 0) this.childElements.splice(index, 1);
    index = this.nestedElements.indexOf(child);
    if (index >= 0) this.nestedElements.splice(index, 1);
    if (child.parent === this) child.parent = undefined;
  }
}

function fakeDocument(globalNonce = '', metaNonce = ''): Document {
  const head = new FakeParent();
  return {
    nodeType: 9,
    head,
    createElement: (name: string) => new FakeElement(name),
    defaultView: globalNonce ? { __webui: { nonce: globalNonce } } : null,
    querySelector: () => metaNonce ? { content: metaNonce } : null,
    querySelectorAll: () => head.querySelectorAll(),
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

  test('a bundled resource covers its member component closures', async () => {
    const document = fakeDocument();
    registerComponentStyles({
      version: 1,
      strategy: 'style',
      resources: {
        '_chunk-first-2': {
          kind: 'style',
          css: '.first{}.second{}',
          members: ['first', 'second'],
        },
        first: { kind: 'style', css: '.first{}' },
        second: { kind: 'style', css: '.second{}' },
      },
      closures: {
        root: ['_chunk-first-2'],
        first: ['first'],
        second: ['second'],
      },
    }, document);

    await installComponentStyles('root', document);
    await installComponentStyles('first', document);
    await installComponentStyles('second', document);

    assert.deepEqual(
      (document.head as unknown as FakeParent).children
        .map(child => child.getAttribute('data-webui-resource')),
      ['_chunk-first-2'],
    );
  });

  test('claims nested SSR route styles before a descendant installs', async () => {
    const document = fakeDocument();
    const root = fakeShadow(document);
    const parent = root as unknown as FakeParent;
    for (const id of ['first', 'second']) {
      const marker = new FakeElement('style');
      marker.setAttribute('data-webui-resource', id);
      marker.setAttribute('data-webui-strategy', 'style');
      parent.appendNested(marker);
    }
    registerComponentStyles(styles({
      route: ['first', 'second'],
      descendant: ['second'],
    }), document);

    await installComponentStyles('descendant', root);
    await installComponentStyles('route', root);

    assert.equal(parent.children.length, 0, 'nested SSR markers must prevent direct duplicates');
    assert.deepEqual(
      parent.querySelectorAll().map(child => child.getAttribute('data-webui-resource')),
      ['first', 'second'],
    );
  });

  test('claims route markers added after the CSS tree was first scanned', async () => {
    const document = fakeDocument();
    const root = fakeShadow(document);
    registerComponentStyles(styles({
      empty: [],
      lateRoute: ['first'],
    }), document);
    await installComponentStyles('empty', root);

    const marker = new FakeElement('style');
    marker.setAttribute('data-webui-resource', 'first');
    marker.setAttribute('data-webui-strategy', 'style');
    claimSsrComponentStyles({
      parentElement: {
        localName: 'webui-route',
        children: [marker],
      },
    } as unknown as Element, root);
    await installComponentStyles('lateRoute', root);

    assert.equal(
      (root as unknown as FakeParent).children.length,
      0,
      'a streamed route marker must prevent a direct duplicate',
    );
  });

  test('adopts nested Module markers in route order when a descendant installs first', async () => {
    const document = fakeDocument();
    const root = fakeShadow(document);
    const parent = root as unknown as FakeParent;
    const firstSheet = {} as CSSStyleSheet;
    const secondSheet = {} as CSSStyleSheet;
    for (const id of ['first', 'second']) {
      const marker = new FakeElement('style');
      marker.setAttribute('data-webui-resource', id);
      marker.setAttribute('data-webui-strategy', 'module');
      parent.appendNested(marker);
    }
    registerComponentStyles({
      version: 1,
      strategy: 'module',
      resources: {
        first: { kind: 'module', specifier: 'first', css: '.first{}' },
        second: { kind: 'module', specifier: 'second', css: '.second{}' },
      },
      closures: {
        route: ['first', 'second'],
        descendant: ['second'],
      },
    }, document);
    setCssModuleLoaderForTests((specifier) => Promise.resolve({
      default: specifier === 'first' ? firstSheet : secondSheet,
    }));

    await Promise.all([
      installComponentStyles('descendant', root),
      installComponentStyles('route', root),
    ]);
    setCssModuleLoaderForTests();

    assert.deepEqual(root.adoptedStyleSheets, [firstSheet, secondSheet]);
    assert.equal(parent.querySelectorAll().length, 0, 'adopted SSR fallbacks must be removed');
  });

  test('installs around existing markers synchronously and caches the completed closure', () => {
    const document = fakeDocument();
    const root = fakeShadow(document);
    const parent = root as unknown as FakeParent;
    const second = new FakeElement('style');
    second.setAttribute('data-webui-resource', 'second');
    parent.appendChild(second);
    registerComponentStyles(styles({ root: ['first', 'second'] }), document);

    assert.equal(installComponentStyles('root', root), undefined);
    assert.equal(parent.markerScans, 1);
    assert.equal(installComponentStyles('root', root), undefined);
    assert.equal(parent.markerScans, 1, 'a completed closure should not rescan markers');
    assert.deepEqual(
      parent.children.map(child => child.getAttribute('data-webui-resource')),
      ['first', 'second'],
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
    const nearMatch = new FakeElement('style');
    nearMatch.setAttribute('data-webui-resource', 'first-child');
    const exact = new FakeElement('style');
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

    assert.doesNotThrow(() => registerComponentStyles({
      version: 1,
      strategy: 'style',
      resources: { first: { css: '.first{}', kind: 'style' } },
      closures: { root: ['first'] },
    }, document));
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
    const marker = new FakeElement('style');
    marker.setAttribute('data-webui-resource', 'module');
    (root as unknown as FakeParent).appendChild(marker);
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

    const failedInstall = installComponentStyles('root', root);
    assert.ok(failedInstall);
    await assert.rejects(failedInstall);
    assert.equal(marker.removed, false);
    const retry = installComponentStyles('root', root);
    assert.ok(retry);
    await retry;
    setCssModuleLoaderForTests();

    assert.equal(attempts, 2);
    assert.deepEqual(root.adoptedStyleSheets, [unrelated, componentSheet]);
    assert.equal(marker.removed, true);

    const importMaps = (document.head as unknown as FakeParent).children.filter(
      (child) => (child as unknown as { type: string }).type === 'importmap',
    );
    assert.equal(importMaps.length, 1);
  });

  test('keeps a successful Module prefix and retries only the rejected suffix', async () => {
    const document = fakeDocument();
    const root = fakeShadow(document);
    const dependencySheet = {} as CSSStyleSheet;
    const componentSheet = {} as CSSStyleSheet;
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
    let componentAttempts = 0;
    setCssModuleLoaderForTests((specifier) => {
      loaded.push(specifier);
      if (specifier === 'dependency') {
        return Promise.resolve({ default: dependencySheet });
      }
      componentAttempts++;
      return componentAttempts === 1
        ? Promise.reject(new Error('component failed'))
        : Promise.resolve({ default: componentSheet });
    });

    const failedInstall = installComponentStyles('component', root);
    assert.ok(failedInstall);
    await assert.rejects(failedInstall);
    assert.deepEqual(root.adoptedStyleSheets, [dependencySheet]);

    const retry = installComponentStyles('component', root);
    assert.ok(retry);
    await retry;
    setCssModuleLoaderForTests();

    assert.deepEqual(loaded, ['dependency', 'component', 'component']);
    assert.deepEqual(root.adoptedStyleSheets, [dependencySheet, componentSheet]);
  });

  test('adopts concurrent closures in reserved order when a descendant resolves first', async () => {
    const document = fakeDocument();
    const root = fakeShadow(document);
    const parentSheet = {} as CSSStyleSheet;
    const childSheet = {} as CSSStyleSheet;
    registerComponentStyles({
      version: 1,
      strategy: 'module',
      resources: {
        parent: { kind: 'module', specifier: 'parent', css: '.parent{}' },
        child: { kind: 'module', specifier: 'child', css: '.child{}' },
      },
      closures: { parent: ['parent', 'child'], child: ['child'] },
    }, document);
    let releaseParent: (() => void) | undefined;
    const parentGate = new Promise<void>((resolve) => {
      releaseParent = resolve;
    });
    setCssModuleLoaderForTests((specifier) =>
      specifier === 'parent'
        ? parentGate.then(() => ({ default: parentSheet }))
        : Promise.resolve({ default: childSheet }));

    // The parent reserves both slots first, then the child closure resolves
    // ahead of it.
    const parentInstall = installComponentStyles('parent', root);
    const childInstall = installComponentStyles('child', root);
    await childInstall;
    releaseParent?.();
    await parentInstall;
    setCssModuleLoaderForTests();

    assert.deepEqual(root.adoptedStyleSheets, [parentSheet, childSheet]);
  });

  test('releases adopted Module fallback markers', async () => {
    const document = fakeDocument();
    const root = fakeShadow(document);
    const parent = root as unknown as FakeParent;
    const sheet = {} as CSSStyleSheet;
    const marker = new FakeElement('style');
    marker.setAttribute('data-webui-resource', 'component');
    marker.setAttribute('data-webui-strategy', 'module');
    parent.appendNested(marker);
    registerComponentStyles({
      version: 1,
      strategy: 'module',
      resources: { component: { kind: 'module', specifier: 'component', css: '.component{}' } },
      closures: { component: ['component'] },
    }, document);
    setCssModuleLoaderForTests(() => Promise.resolve({ default: sheet }));

    await installComponentStyles('component', root);
    setCssModuleLoaderForTests();

    assert.equal(marker.removed, true);
    assert.equal(marker.parent, undefined, 'adopted marker must not stay reachable');
    assert.deepEqual(root.adoptedStyleSheets, [sheet]);
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

  test('starts Module loads concurrently, dedupes pending loads, and adopts once in closure order', async () => {
    const document = fakeDocument();
    const root = fakeShadow(document);
    const dependencySheet = {} as CSSStyleSheet;
    const componentSheet = {} as CSSStyleSheet;
    registerComponentStyles({
      version: 1,
      strategy: 'module',
      resources: {
        dependency: { kind: 'module', specifier: 'dependency', css: '.dependency{}' },
        component: { kind: 'module', specifier: 'component', css: '.component{}' },
      },
      closures: { component: ['dependency', 'component'] },
    }, document);

    let adopted: CSSStyleSheet[] = [];
    let assignments = 0;
    Object.defineProperty(root, 'adoptedStyleSheets', {
      get: () => adopted,
      set: (value: CSSStyleSheet[]) => {
        assignments++;
        adopted = value;
      },
      configurable: true,
    });
    const loaded: string[] = [];
    const resolvers = new Map<
      string,
      (module: { default: CSSStyleSheet }) => void
    >();
    setCssModuleLoaderForTests((specifier) => {
      loaded.push(specifier);
      return new Promise(resolve => {
        resolvers.set(specifier, resolve);
      });
    });

    const firstInstall = installComponentStyles('component', root);
    const duplicateInstall = installComponentStyles('component', root);
    assert.ok(firstInstall);
    assert.ok(duplicateInstall);
    assert.deepEqual(loaded, ['dependency', 'component']);

    const resolveComponent = resolvers.get('component');
    const resolveDependency = resolvers.get('dependency');
    assert.ok(resolveComponent);
    assert.ok(resolveDependency);
    resolveComponent({ default: componentSheet });
    await Promise.resolve();
    assert.deepEqual(adopted, []);
    resolveDependency({ default: dependencySheet });

    await Promise.all([firstInstall, duplicateInstall]);
    setCssModuleLoaderForTests();

    assert.deepEqual(adopted, [dependencySheet, componentSheet]);
    assert.equal(assignments, 1);
    assert.equal(installComponentStyles('component', root), undefined);
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
