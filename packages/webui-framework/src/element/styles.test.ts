// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test, beforeEach, afterEach } from 'node:test';

/** Minimal CSSStyleSheet shim for Node.js tests. */
class FakeCSSStyleSheet {
  cssText = '';
  replaceSync(text: string) {
    this.cssText = text;
  }
}

function makeStyleDef(specifier: string, css: string) {
  return {
    type: 'module',
    specifier,
    textContent: css,
    getAttribute(name: string) {
      if (name === 'specifier') return this.specifier;
      if (name === 'type') return this.type;
      return null;
    },
  };
}

describe('injectModuleStyle', () => {
  let prevDocument: unknown;
  let prevCSSStyleSheet: unknown;
  let headAppendedStyles: Array<{ textContent: string }>;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let styleDefs: any[];

  beforeEach(() => {
    prevDocument = (globalThis as any).document;
    prevCSSStyleSheet = (globalThis as any).CSSStyleSheet;
    headAppendedStyles = [];
    // Each test uses unique specifiers to avoid cross-test cache pollution
    styleDefs = [];

    (globalThis as any).CSSStyleSheet = FakeCSSStyleSheet;
    (globalThis as any).document = {
      querySelectorAll(selector: string) {
        if (selector === 'style[type="module"][specifier]') {
          return Object.assign([...styleDefs], { length: styleDefs.length });
        }
        return Object.assign([], { length: 0 });
      },
      createElement(tag: string) {
        return { tagName: tag, textContent: '' };
      },
      head: {
        appendChild(el: { textContent: string }) {
          headAppendedStyles.push(el);
          return el;
        },
      },
    };
  });

  afterEach(() => {
    (globalThis as any).document = prevDocument;
    (globalThis as any).CSSStyleSheet = prevCSSStyleSheet;
  });

  test('creates CSSStyleSheet and adopts onto shadow root', async () => {
    styleDefs.push(makeStyleDef('adopt-test-1', '.a{color:red}'));
    const { injectModuleStyle } = await import('./styles.js');

    const sr = { adoptedStyleSheets: [] as FakeCSSStyleSheet[] };
    injectModuleStyle('adopt-test-1', sr as unknown as ShadowRoot);

    assert.equal(sr.adoptedStyleSheets.length, 1);
    assert.equal(sr.adoptedStyleSheets[0].cssText, '.a{color:red}');
  });

  test('reuses cached sheet for second shadow root', async () => {
    styleDefs.push(makeStyleDef('cache-test-1', '.b{color:blue}'));
    const { injectModuleStyle } = await import('./styles.js');

    const sr1 = { adoptedStyleSheets: [] as FakeCSSStyleSheet[] };
    const sr2 = { adoptedStyleSheets: [] as FakeCSSStyleSheet[] };

    injectModuleStyle('cache-test-1', sr1 as unknown as ShadowRoot);
    injectModuleStyle('cache-test-1', sr2 as unknown as ShadowRoot);

    assert.equal(sr1.adoptedStyleSheets.length, 1);
    assert.equal(sr2.adoptedStyleSheets.length, 1);
    assert.equal(
      sr1.adoptedStyleSheets[0],
      sr2.adoptedStyleSheets[0],
      'both shadow roots should share the same CSSStyleSheet instance',
    );
  });

  test('does not duplicate adoption on same shadow root', async () => {
    styleDefs.push(makeStyleDef('dedup-test-1', '.c{color:green}'));
    const { injectModuleStyle } = await import('./styles.js');

    const sr = { adoptedStyleSheets: [] as FakeCSSStyleSheet[] };
    injectModuleStyle('dedup-test-1', sr as unknown as ShadowRoot);
    injectModuleStyle('dedup-test-1', sr as unknown as ShadowRoot);

    assert.equal(sr.adoptedStyleSheets.length, 1);
  });

  test('appends style to head for light DOM (null shadow root)', async () => {
    styleDefs.push(makeStyleDef('light-test-1', '.d{color:pink}'));
    const { injectModuleStyle } = await import('./styles.js');

    injectModuleStyle('light-test-1', null);

    assert.equal(headAppendedStyles.length, 1);
    assert.equal(headAppendedStyles[0].textContent, '.d{color:pink}');
  });

  test('skips silently when specifier not found', async () => {
    // styleDefs is empty — no definitions
    const { injectModuleStyle } = await import('./styles.js');

    const sr = { adoptedStyleSheets: [] as FakeCSSStyleSheet[] };
    injectModuleStyle('nonexistent-spec', sr as unknown as ShadowRoot);

    assert.equal(sr.adoptedStyleSheets.length, 0);
    assert.equal(headAppendedStyles.length, 0);
  });
});
