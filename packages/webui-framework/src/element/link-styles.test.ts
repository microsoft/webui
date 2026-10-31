// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test, type TestContext } from 'node:test';

import {
  prepareComponentStyleLinks,
  prepareComponentStyleRoots,
  prepareRegisteredLinkStyles,
  prepareTemplateLinkStyles,
  preloadComponentAssetStyles,
  rewriteCssUrls,
  templateMayContainAuthorStyles,
  templateMayContainLinkStyles,
} from './link-styles.js';
import { registerPreparedComponentStyles } from './style-catalog.js';
import { speculativeTemplates } from '../speculation.js';
import type { TemplateMeta } from '../template-types.js';

type GlobalName = 'CSSStyleSheet' | 'ShadowRoot' | 'document' | 'window';

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

describe('Link stylesheet scanning', () => {
  test('skips Link-mode mount work for templates without link elements', () => {
    assert.equal(
      templateMayContainLinkStyles({ h: '<p class="card">Ready</p>' }),
      false,
    );
    assert.equal(
      templateMayContainLinkStyles({ h: '<link-card></link-card>' }),
      false,
    );
  });

  test('finds author styles in any template block', () => {
    assert.equal(
      templateMayContainAuthorStyles({ v: 3, h: '<p class="card">Ready</p>' }),
      false,
    );
    assert.equal(
      templateMayContainAuthorStyles({ v: 3, h: '<style-card></style-card><link-card></link-card>' }),
      false,
    );
    assert.equal(
      templateMayContainAuthorStyles({ v: 3, h: '<STYLE>.a{}</STYLE><p></p>' }),
      true,
    );
    assert.equal(
      templateMayContainAuthorStyles({
        v: 3,
        h: '<p></p>',
        b: [{ h: '<span></span>' }, { h: '<link rel="stylesheet" href="x.css">' }],
      }),
      true,
    );
  });

  test('returns rules without URL candidates unchanged', () => {
    const href = 'https://cdn.example.test/components/card.css';
    const css = '.a{color:rgb(1,2,3);margin:calc(1px + 2px);width:var(--w)}';
    assert.equal(rewriteCssUrls(css, href), css);
    assert.equal(rewriteCssUrls('.a{content:"\\2014"}', href), '.a{content:"\\2014"}');
    assert.equal(
      rewriteCssUrls('/* url(old.svg) */.a{color:red}', href),
      '/* url(old.svg) */.a{color:red}',
    );
  });

  test('still scans mixed-case URL functions and escapes', () => {
    const href = 'https://cdn.example.test/components/card.css';
    assert.equal(
      rewriteCssUrls('.a{width:calc(1px);--icon:URL(icons/a.svg)}', href),
      '.a{width:calc(1px);--icon:URL("https://cdn.example.test/components/icons/a.svg")}',
    );
    assert.equal(
      rewriteCssUrls('.a{background:-WEBKIT-IMAGE-SET("a.png" 1x)}', href),
      undefined,
    );
    assert.equal(rewriteCssUrls('.a\\:hover{color:red}', href), undefined);
  });

  test('resolves quoted and unquoted URLs against the stylesheet', () => {
    assert.equal(
      rewriteCssUrls(
        '.a{background:url(../icons/a.svg)}.b{mask:url("./b.svg#mask")}',
        'https://cdn.example.test/components/card/card.css',
      ),
      '.a{background:url("https://cdn.example.test/components/icons/a.svg")}' +
      '.b{mask:url("https://cdn.example.test/components/card/b.svg#mask")}',
    );
    assert.equal(
      rewriteCssUrls(
        ".a{background:url('icon.svg')}",
        "https://cdn.example.test/components/card's/card.css",
      ),
      ".a{background:url('https://cdn.example.test/components/card\\'s/icon.svg')}",
    );
  });

  test('preserves fragment URLs and ignores URL text in strings or comments', () => {
    const css = '.a{filter:url(#blur);content:"url(fake.svg)"}/* url(old.svg) */';
    assert.equal(
      rewriteCssUrls(css, 'https://cdn.example.test/components/card.css'),
      css,
    );
  });

  test('falls back for CSS URL forms that cannot be safely rewritten', () => {
    assert.equal(
      rewriteCssUrls(
        '.a{background:image-set("small.png" 1x,"large.png" 2x)}',
        'https://cdn.example.test/components/card.css',
      ),
      undefined,
    );
    assert.equal(
      rewriteCssUrls(
        '.a{background:url(var(--image))}',
        'https://cdn.example.test/components/card.css',
      ),
      undefined,
    );
    assert.equal(
      rewriteCssUrls(
        '.a{background:u\\72l(escaped.png)}',
        'https://cdn.example.test/components/card.css',
      ),
      undefined,
    );
  });

  test('allows a speculative href to preload again after TTL cleanup', async (context) => {
    class MockCssStyleSheet {
      replaceSync(): void {}
    }
    class MockShadowRoot {}
    Object.defineProperty(MockShadowRoot.prototype, 'adoptedStyleSheets', {
      value: [],
      configurable: true,
      writable: true,
    });

    const appended: Array<{ onload: (() => void) | null }> = [];
    const previousCssStyleSheet = setGlobal('CSSStyleSheet', MockCssStyleSheet);
    const previousShadowRoot = setGlobal('ShadowRoot', MockShadowRoot);
    context.mock.timers.enable({ apis: ['setTimeout'] });
    const previousDocument = setGlobal('document', {
      baseURI: 'https://example.test/app/',
      querySelector: () => null,
      createElement() {
        return {
          as: '',
          crossOrigin: '',
          href: '',
          integrity: '',
          onerror: null,
          onload: null,
          referrerPolicy: '',
          rel: '',
          remove() {},
        };
      },
      head: {
        appendChild(link: { onload: (() => void) | null }) {
          appended.push(link);
          link.onload?.();
          return link;
        },
      },
    });

    try {
      const href = '/retry.css';
      preloadComponentAssetStyles([href]);
      await Promise.resolve();
      context.mock.timers.tick(3_000);

      preloadComponentAssetStyles([href]);
      assert.equal(appended.length, 2);
      await Promise.resolve();
      context.mock.timers.tick(3_000);
    } finally {
      restoreGlobal('CSSStyleSheet', previousCssStyleSheet);
      restoreGlobal('ShadowRoot', previousShadowRoot);
      restoreGlobal('document', previousDocument);
    }
  });

  test('does not duplicate an SSR stylesheet preload', () => {
    class MockCssStyleSheet {
      replaceSync(): void {}
    }
    class MockShadowRoot {}
    Object.defineProperty(MockShadowRoot.prototype, 'adoptedStyleSheets', {
      value: [],
      configurable: true,
      writable: true,
    });

    const previousCssStyleSheet = setGlobal('CSSStyleSheet', MockCssStyleSheet);
    const previousShadowRoot = setGlobal('ShadowRoot', MockShadowRoot);
    const previousDocument = setGlobal('document', {
      baseURI: 'https://example.test/app/',
      querySelector: () => null,
      createElement() {
        return {
          as: '',
          crossOrigin: '',
          href: '',
          integrity: '',
          onerror: null,
          onload: null,
          referrerPolicy: '',
          rel: '',
          remove() {},
        };
      },
      head: {
        querySelectorAll() {
          return [{ href: 'https://example.test/dashboard.css' }];
        },
        appendChild() {
          assert.fail('an existing SSR preload should be reused');
        },
      },
    });

    try {
      assert.equal(
        preloadComponentAssetStyles(['/dashboard.css']),
        undefined,
      );
    } finally {
      restoreGlobal('CSSStyleSheet', previousCssStyleSheet);
      restoreGlobal('ShadowRoot', previousShadowRoot);
      restoreGlobal('document', previousDocument);
    }
  });

  test('catalog stylesheets preload from the runtime asset base', async (context) => {
    class MockCssStyleSheet {
      replaceSync(): void {}
    }
    class MockShadowRoot {}
    Object.defineProperty(MockShadowRoot.prototype, 'adoptedStyleSheets', {
      value: [],
      configurable: true,
      writable: true,
    });

    const appended: Array<{ href: string; onload: (() => void) | null }> = [];
    const previousCssStyleSheet = setGlobal('CSSStyleSheet', MockCssStyleSheet);
    const previousShadowRoot = setGlobal('ShadowRoot', MockShadowRoot);
    context.mock.timers.enable({ apis: ['setTimeout'] });
    const previousDocument = setGlobal('document', {
      baseURI: 'https://site.example/news/',
      defaultView: { __webui: { assetBase: 'https://cdn.example/assets/' } },
      createElement() {
        return { as: '', href: '', onerror: null, onload: null, rel: '', remove() {} };
      },
      head: {
        appendChild(link: { href: string; onload: (() => void) | null }) {
          appended.push(link);
          link.onload?.();
          return link;
        },
      },
    });

    try {
      const ready = prepareComponentStyleLinks({
        version: 1,
        strategy: 'link',
        resources: {
          card: { kind: 'link', href: 'app-card.css' },
          shell: { kind: 'link', href: 'https://fonts.example/shell.css' },
        },
        closures: { card: ['card', 'shell'] },
      });
      assert.deepEqual(appended.map(link => link.href), [
        'https://cdn.example/assets/app-card.css',
        'https://fonts.example/shell.css',
      ]);
      await ready;
      context.mock.timers.tick(3_000);
    } finally {
      restoreGlobal('CSSStyleSheet', previousCssStyleSheet);
      restoreGlobal('ShadowRoot', previousShadowRoot);
      restoreGlobal('document', previousDocument);
    }
  });
});

interface FakePreload {
  href: string;
  fetchPriority?: string;
  onload: (() => void) | null;
}

/**
 * A document that records stylesheet preloads without loading them. Its
 * `<template>` parses only the first `href` of the HTML it is given into one
 * stylesheet `<link>`.
 */
function preloadDocument(appended: FakePreload[]): { readyState: DocumentReadyState } {
  const page = {
    readyState: 'complete' as DocumentReadyState,
    baseURI: 'https://example.test/app/',
    querySelector: () => null,
    createElement(tag: string) {
      if (tag !== 'template') return { as: '', href: '', onerror: null, onload: null, rel: '', remove() {} };
      let href = '';
      const link = {
        localName: 'link',
        relList: { contains: (token: string) => token === 'stylesheet' },
        getAttribute: (name: string) => (name === 'href' ? href : null),
        attributes: [{ name: 'rel' }, { name: 'href' }],
        disabled: false,
        integrity: '',
        media: '',
        referrerPolicy: '',
        title: '',
        type: '',
      };
      return {
        set innerHTML(html: string) { href = /href="([^"]+)"/.exec(html)?.[1] ?? ''; },
        content: { querySelectorAll: () => [link], querySelector: () => null },
      };
    },
    head: {
      appendChild(link: FakePreload) {
        appended.push(link);
        return link;
      },
    },
  };
  return page;
}

describe('Speculative stylesheet preload priority', () => {
  class MockCssStyleSheet {
    replaceSync(): void {}
  }
  class MockShadowRoot {}
  Object.defineProperty(MockShadowRoot.prototype, 'adoptedStyleSheets', {
    value: [],
    configurable: true,
    writable: true,
  });

  async function withPreloadDocument(
    context: TestContext,
    run: (appended: FakePreload[], page: { readyState: DocumentReadyState }) => Promise<void> | void,
  ): Promise<void> {
    const appended: FakePreload[] = [];
    const page = preloadDocument(appended);
    // The template parser reads the window's Trusted Types support.
    const previous = [
      ['CSSStyleSheet', setGlobal('CSSStyleSheet', MockCssStyleSheet)],
      ['ShadowRoot', setGlobal('ShadowRoot', MockShadowRoot)],
      ['document', setGlobal('document', page)],
      ['window', setGlobal('window', new EventTarget())],
    ] as const;
    context.mock.timers.enable({ apis: ['setTimeout'] });
    try {
      await run(appended, page);
    } finally {
      for (const link of appended) link.onload?.();
      context.mock.timers.tick(3_000);
      for (const [name, descriptor] of previous) restoreGlobal(name, descriptor);
    }
  }

  test('low priority marks new preloads, and later requests share them', async (context) => {
    await withPreloadDocument(context, async (appended) => {
      const speculative = preloadComponentAssetStyles(['region.css'], 'low');
      assert.deepEqual(appended.map(link => [link.href, link.fetchPriority]), [
        ['https://example.test/app/region.css', 'low'],
      ]);
      const registration = preloadComponentAssetStyles(['region.css']);
      assert.equal(appended.length, 1, 'the pack registration joins the speculative request');
      preloadComponentAssetStyles(['active.css'], 'high');
      assert.equal(appended[1].fetchPriority, undefined, 'activation keeps the default priority');
      appended[0].onload?.();
      await Promise.all([speculative, registration]);
    });
  });

  test('style roots preload their catalog closure at the given priority', async (context) => {
    await withPreloadDocument(context, (appended) => {
      registerPreparedComponentStyles({
        version: 1,
        strategy: 'link',
        resources: { 'region-card': { kind: 'link', href: 'region-card.css' } },
        closures: { 'region-card': ['region-card'] },
      });
      void prepareComponentStyleRoots(['region-card'], 'low');
      assert.deepEqual(appended.map(link => [link.href, link.fetchPriority]), [
        ['https://example.test/app/region-card.css', 'low'],
      ]);
    });
  });

  test('before the window loads, speculative preloads wait for it and nothing waits for them', async (context) => {
    await withPreloadDocument(context, (appended, page) => {
      page.readyState = 'interactive';
      registerPreparedComponentStyles({
        version: 1,
        strategy: 'link',
        resources: { 'held-card': { kind: 'link', href: 'held-card.css' } },
        closures: { 'held-card': ['held-card'] },
      });
      assert.equal(preloadComponentAssetStyles(['region.css'], 'low'), undefined);
      assert.equal(prepareComponentStyleRoots(['held-card'], 'low'), undefined);
      assert.equal(appended.length, 0, 'nothing speculative starts while the window loads');
      void preloadComponentAssetStyles(['active.css'], 'high');
      assert.deepEqual(appended.map(link => [link.href, link.fetchPriority]), [
        ['https://example.test/app/active.css', undefined],
      ], 'an activation starts at once');

      page.readyState = 'complete';
      window.dispatchEvent(new Event('load'));
      assert.deepEqual(appended.map(link => [link.href, link.fetchPriority]).slice(1), [
        ['https://example.test/app/region.css', 'low'],
        ['https://example.test/app/held-card.css', 'low'],
      ]);
    });
  });

  /** A template whose only stylesheet is `href`. */
  function linked(href: string): TemplateMeta {
    return { v: 3, h: `<link rel="stylesheet" href="${href}"><p></p>` };
  }

  test('after the window loads, templates a speculative lazy load registers preload their stylesheets at low priority', async (context) => {
    await withPreloadDocument(context, (appended) => {
      const speculative = linked('owned.css');
      const declared = linked('declared.css');
      const active = linked('active.css');
      speculativeTemplates.add(speculative);
      speculativeTemplates.add(declared);
      // Registration prepares the record; a definition that was waiting for
      // its template prepares that template directly.
      void prepareRegisteredLinkStyles({ 'owned-card': speculative });
      void prepareTemplateLinkStyles(declared);
      void prepareTemplateLinkStyles(active);
      assert.deepEqual(appended.map(link => [link.href, link.fetchPriority]), [
        ['https://example.test/app/owned.css', 'low'],
        ['https://example.test/app/declared.css', 'low'],
        ['https://example.test/app/active.css', undefined],
      ]);
    });
  });

  test('before the window loads, nothing waits for a speculative template stylesheet and none starts', async (context) => {
    await withPreloadDocument(context, (appended, page) => {
      page.readyState = 'interactive';
      const speculative = linked('owned.css');
      const declared = linked('declared.css');
      const mounted = linked('mounted.css');
      const plain: TemplateMeta = { v: 3, h: '<p></p>' };
      for (const meta of [speculative, declared, mounted, plain]) speculativeTemplates.add(meta);
      assert.equal(prepareRegisteredLinkStyles({ 'owned-card': speculative, 'plain-card': plain }), undefined);
      assert.equal(prepareTemplateLinkStyles(declared), undefined);
      void prepareTemplateLinkStyles(mounted);
      assert.equal(appended.length, 0, 'speculative stylesheets wait for the window load event');

      // A region that activates first prepares the stylesheets it mounts at once.
      assert.equal(templateMayContainLinkStyles(mounted), true);
      assert.deepEqual(appended.map(link => [link.href, link.fetchPriority]), [
        ['https://example.test/app/mounted.css', undefined],
      ]);

      page.readyState = 'complete';
      window.dispatchEvent(new Event('load'));
      assert.deepEqual(appended.map(link => [link.href, link.fetchPriority]).slice(1), [
        ['https://example.test/app/owned.css', 'low'],
        ['https://example.test/app/declared.css', 'low'],
      ], 'the rest start once the window has loaded, and a mounted template does not start twice');
    });
  });
});
