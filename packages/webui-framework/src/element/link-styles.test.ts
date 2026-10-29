// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';

import {
  preloadComponentAssetStyles,
  rewriteCssUrls,
  templateMayContainAuthorStyles,
  templateMayContainLinkStyles,
} from './link-styles.js';

type GlobalName = 'CSSStyleSheet' | 'ShadowRoot' | 'document';

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
      templateMayContainAuthorStyles({ h: '<p class="card">Ready</p>' }),
      false,
    );
    assert.equal(
      templateMayContainAuthorStyles({ h: '<style-card></style-card><link-card></link-card>' }),
      false,
    );
    assert.equal(
      templateMayContainAuthorStyles({ h: '<STYLE>.a{}</STYLE><p></p>' }),
      true,
    );
    assert.equal(
      templateMayContainAuthorStyles({
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
});
