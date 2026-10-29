// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import {
  registerComponentStyles,
  registerTemplateData,
  WebUIElement,
  type ComponentStyles,
} from '../../../src/index.js';

const STYLESHEETS: Record<string, string> = {
  'test-shared-link-card': '/component-link-sharing/shared.css',
  'test-cold-link-card': '/component-link-sharing/cold.css',
  'test-import-link-card': '/component-link-sharing/import.css',
  'test-styled-link-card': '/component-link-sharing/styled.css',
};

interface LinkCardReport {
  readonly activeLinks: number;
  readonly adopted: number;
  readonly cards: number;
  readonly colors: string[];
  readonly distinctSheets: number;
  readonly guards: number;
  readonly hidden: number;
  readonly links: number;
  readonly sameSheetAsFirstBatch: boolean;
}

declare global {
  interface Window {
    __inspectLinkCards?: (tag: string) => LinkCardReport;
    __mountLinkCards?: (tag: string, count: number) => void;
    __registerLinkCards?: (tag: string, prepare: boolean) => Promise<void> | undefined;
    __removeLinkCards?: (tag: string) => void;
  }
}

class TestSharedLinkCard extends WebUIElement {}
class TestColdLinkCard extends WebUIElement {}
class TestImportLinkCard extends WebUIElement {}
class TestStyledLinkCard extends WebUIElement {}

registerTemplateData({
  'test-shared-link-card': { h: '<p class="label">Shared</p>', sd: 1 },
  'test-cold-link-card': { h: '<p class="label">Cold</p>', sd: 1 },
  'test-import-link-card': { h: '<p class="label">Import</p>', sd: 1 },
  'test-styled-link-card': {
    h: '<style>.label { font-weight: 700; }</style><p class="label">Styled</p>',
    sd: 1,
  },
});
TestSharedLinkCard.define('test-shared-link-card');
TestColdLinkCard.define('test-cold-link-card');
TestImportLinkCard.define('test-import-link-card');
TestStyledLinkCard.define('test-styled-link-card');

function linkCatalog(tag: string): ComponentStyles {
  return {
    version: 1,
    strategy: 'link',
    resources: { [tag]: { kind: 'link', href: STYLESHEETS[tag] } },
    closures: { [tag]: [tag] },
  };
}

const firstSheets = new Map<string, CSSStyleSheet>();

window.__registerLinkCards = (tag, prepare) => {
  // The router and component assets register through this hook, which also
  // warms the stylesheet. Plain registration models a cold client mount.
  if (prepare) return window.__webuiRegisterComponentStyles?.(linkCatalog(tag));
  registerComponentStyles(linkCatalog(tag));
  return undefined;
};

window.__mountLinkCards = (tag, count) => {
  const container = document.getElementById('cards');
  if (!container) throw new Error('Missing #cards container');
  const fragment = document.createDocumentFragment();
  for (let i = 0; i < count; i++) fragment.append(document.createElement(tag));
  container.append(fragment);
};

window.__removeLinkCards = (tag) => {
  for (const card of document.querySelectorAll(tag)) card.remove();
};

window.__inspectLinkCards = (tag) => {
  const sheets = new Set<CSSStyleSheet>();
  const colors = new Set<string>();
  let activeLinks = 0;
  let adopted = 0;
  let guards = 0;
  let hidden = 0;
  let links = 0;
  let sameSheetAsFirstBatch = true;
  const cards = document.querySelectorAll(tag);
  for (const card of cards) {
    const root = card.shadowRoot;
    if (!root) throw new Error(`Missing shadow root for <${tag}>`);
    for (const link of root.querySelectorAll('link')) {
      links += 1;
      if (!link.disabled) activeLinks += 1;
    }
    for (const style of root.querySelectorAll('style')) {
      if (style.textContent?.includes(':host')) guards += 1;
    }
    adopted += root.adoptedStyleSheets.length;
    for (const sheet of root.adoptedStyleSheets) {
      sheets.add(sheet);
      const first = firstSheets.get(tag);
      if (!first) firstSheets.set(tag, sheet);
      else if (first !== sheet) sameSheetAsFirstBatch = false;
    }
    if (!card.checkVisibility({ visibilityProperty: true })) hidden += 1;
    const label = root.querySelector('.label');
    colors.add(label ? getComputedStyle(label).color : 'missing');
  }
  return {
    activeLinks,
    adopted,
    cards: cards.length,
    colors: [...colors],
    distinctSheets: sheets.size,
    guards,
    hidden,
    links,
    sameSheetAsFirstBatch,
  };
};
