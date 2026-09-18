// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import {
  getTemplateOutlets,
  templateHasRootOutlet,
  templateHtmlMayContainLink,
} from './template-content.js';
import {
  getTemplate,
  registerTemplateData,
  templateHasFragments,
  templateNeedsRanges,
  type TemplateMeta,
} from './template.js';

class TemplateNode {
  readonly nodeType: number;
  readonly firstChild: TemplateNode | null;
  nextSibling: TemplateNode | null = null;
  parentNode: TemplateNode | null = null;

  constructor(readonly localName: string, children: TemplateNode[] = []) {
    this.nodeType = localName ? 1 : 11;
    this.firstChild = children[0] ?? null;
    for (let i = 0; i < children.length; i++) {
      children[i].parentNode = this;
      children[i].nextSibling = children[i + 1] ?? null;
    }
  }
}

test('link prefilter accepts only case-insensitive tag names and HTML delimiters', () => {
  for (const html of ['<link', '<LiNk>', '<link/>', '<link href=x>', '<link\t>', '<link\n>', '<link\r>', '<link\f>']) {
    assert.equal(templateHtmlMayContainLink(html), true, html);
  }
  for (const html of ['', 'link', '</link>', '<links>', '<link-x>', '<link=x>', '< link>', '<link\0>']) {
    assert.equal(templateHtmlMayContainLink(html), false, html);
  }
});

test('ordinary and root-only outlet metadata does not create a range controller or parse DOM', t => {
  const previousWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
  const previousDocument = Object.getOwnPropertyDescriptor(globalThis, 'document');
  t.after(() => {
    if (previousWindow) Object.defineProperty(globalThis, 'window', previousWindow);
    else Reflect.deleteProperty(globalThis, 'window');
    if (previousDocument) Object.defineProperty(globalThis, 'document', previousDocument);
    else Reflect.deleteProperty(globalThis, 'document');
  });
  Object.defineProperty(globalThis, 'window', { configurable: true, value: { __webui: {} } });
  Object.defineProperty(globalThis, 'document', {
    configurable: true,
    value: { createElement() { assert.fail('no template DOM should be parsed'); } },
  });
  const ordinary: TemplateMeta = {
    h: '<p>ordinary</p>',
    b: [{ h: '<outlet-other></outlet-other>' }],
  };
  const root: TemplateMeta = { h: '<outlet />' };
  registerTemplateData({ ordinary, root });
  for (const name of ['ordinary', 'root']) {
    const meta = getTemplate(name);
    assert.ok(meta);
    assert.equal(templateNeedsRanges(meta), false);
    assert.equal(templateHasFragments(meta), false);
  }
  for (const html of ['', '<outlets>', '<outlet-x>', '</outlet>', '&lt;outlet&gt;']) {
    assert.equal(templateHasRootOutlet({ h: html }), false, html);
  }
});

test('outlet indexes are cached once and only actual top-level block outlets need ranges', t => {
  const previousWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
  const previousDocument = Object.getOwnPropertyDescriptor(globalThis, 'document');
  t.after(() => {
    if (previousWindow) Object.defineProperty(globalThis, 'window', previousWindow);
    else Reflect.deleteProperty(globalThis, 'window');
    if (previousDocument) Object.defineProperty(globalThis, 'document', previousDocument);
    else Reflect.deleteProperty(globalThis, 'document');
  });
  Object.defineProperty(globalThis, 'window', { configurable: true, value: { __webui: {} } });
  const dom = new Map([
    ['<OuTlEt />', new TemplateNode('', [new TemplateNode('outlet')])],
    ['<div><outlet /></div>', new TemplateNode('', [
      new TemplateNode('div', [new TemplateNode('outlet')]),
    ])],
    ['<!-- <outlet> -->', new TemplateNode('')],
  ]);
  let parses = 0;
  Object.defineProperty(globalThis, 'document', {
    configurable: true,
    value: {
      createElement(tag: string) {
        assert.equal(tag, 'template');
        let content: TemplateNode | undefined;
        return {
          set innerHTML(html: string) {
            parses++;
            content = dom.get(html);
            assert.ok(content, html);
          },
          get content() { return content; },
        };
      },
    },
  });

  const block = { h: '<OuTlEt />' };
  const wrapped = { h: '<div><outlet /></div>' };
  const comment = { h: '<!-- <outlet> -->' };
  assert.equal(templateHasRootOutlet(block), true);
  const indexes = getTemplateOutlets(block);
  assert.deepEqual(indexes, [1]);
  assert.equal(getTemplateOutlets(block), indexes);
  assert.equal(templateHasRootOutlet(wrapped), false);
  assert.deepEqual(getTemplateOutlets(wrapped), [2]);
  assert.equal(templateHasRootOutlet(comment), false);
  assert.equal(getTemplateOutlets(comment), undefined);
  assert.equal(parses, 3);

  const ranges: TemplateMeta = { h: '', b: [block] };
  const nested: TemplateMeta = { h: '', b: [wrapped] };
  registerTemplateData({ ranges, nested });
  assert.equal(templateNeedsRanges(getTemplate('ranges')!), true);
  assert.equal(templateHasFragments(ranges), false);
  assert.equal(templateNeedsRanges(getTemplate('nested')!), false);
  assert.equal(parses, 3, 'normalization reuses the parsed content');
});
