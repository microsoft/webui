// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';
import { collectItemMarkers, nextElement, findByOrdinal } from './markers.js';

// ── Mock helpers ────────────────────────────────────────────────
// markers.ts only reads nodeType, data, and nextSibling — lightweight
// mocks suffice without a full DOM.

interface MockNode {
  nodeType: number;
  data?: string;
  nextSibling: MockNode | null;
}

const ELEMENT = 1;
const TEXT = 3;
const COMMENT = 8;

// ── collectItemMarkers ──────────────────────────────────────────

describe('collectItemMarkers', () => {
  test('collects <!--wi--> markers and captures <!--/wr--> end marker', () => {
    // <!--wr--> <!--wi--> <elem/> <!--wi--> <elem/> <!--/wr-->
    const wrStart = { nodeType: COMMENT, data: 'wr', nextSibling: null } as MockNode;
    const wi1 = { nodeType: COMMENT, data: 'wi', nextSibling: null } as MockNode;
    const el1 = { nodeType: ELEMENT, nextSibling: null } as MockNode;
    const wi2 = { nodeType: COMMENT, data: 'wi', nextSibling: null } as MockNode;
    const el2 = { nodeType: ELEMENT, nextSibling: null } as MockNode;
    const wrEnd = { nodeType: COMMENT, data: '/wr', nextSibling: null } as MockNode;

    wrStart.nextSibling = wi1;
    wi1.nextSibling = el1;
    el1.nextSibling = wi2;
    wi2.nextSibling = el2;
    el2.nextSibling = wrEnd;

    const { items, end } = collectItemMarkers(wrStart as unknown as Comment);
    assert.equal(items.length, 2, 'should find 2 item markers');
    assert.strictEqual(items[0], wi1);
    assert.strictEqual(items[1], wi2);
    assert.strictEqual(end, wrEnd, 'should capture end marker');
  });

  test('returns empty items and null end for empty repeat', () => {
    // <!--wr--> <!--/wr-->
    const wrStart = { nodeType: COMMENT, data: 'wr', nextSibling: null } as MockNode;
    const wrEnd = { nodeType: COMMENT, data: '/wr', nextSibling: null } as MockNode;
    wrStart.nextSibling = wrEnd;

    const { items, end } = collectItemMarkers(wrStart as unknown as Comment);
    assert.equal(items.length, 0, 'should find no item markers');
    assert.strictEqual(end, wrEnd, 'should capture end marker');
  });

  test('returns null end when <!--/wr--> is missing', () => {
    // <!--wr--> <!--wi--> <elem/> (no end marker — walks off)
    const wrStart = { nodeType: COMMENT, data: 'wr', nextSibling: null } as MockNode;
    const wi = { nodeType: COMMENT, data: 'wi', nextSibling: null } as MockNode;
    const el = { nodeType: ELEMENT, nextSibling: null } as MockNode;

    wrStart.nextSibling = wi;
    wi.nextSibling = el;

    const { items, end } = collectItemMarkers(wrStart as unknown as Comment);
    assert.equal(items.length, 1);
    assert.strictEqual(end, null, 'end should be null when <!--/wr--> is missing');
  });

  test('skips non-comment siblings', () => {
    // <!--wr--> text elem <!--wi--> text <!--/wr-->
    const wrStart = { nodeType: COMMENT, data: 'wr', nextSibling: null } as MockNode;
    const t1 = { nodeType: TEXT, data: 'hello', nextSibling: null } as MockNode;
    const e1 = { nodeType: ELEMENT, nextSibling: null } as MockNode;
    const wi = { nodeType: COMMENT, data: 'wi', nextSibling: null } as MockNode;
    const t2 = { nodeType: TEXT, data: 'world', nextSibling: null } as MockNode;
    const wrEnd = { nodeType: COMMENT, data: '/wr', nextSibling: null } as MockNode;

    wrStart.nextSibling = t1;
    t1.nextSibling = e1;
    e1.nextSibling = wi;
    wi.nextSibling = t2;
    t2.nextSibling = wrEnd;

    const { items, end } = collectItemMarkers(wrStart as unknown as Comment);
    assert.equal(items.length, 1);
    assert.strictEqual(items[0], wi);
    assert.strictEqual(end, wrEnd);
  });

  test('ignores unrelated comment markers', () => {
    // <!--wr--> <!--wc--> <!--wi--> <!--/wc--> <!--/wr-->
    const wrStart = { nodeType: COMMENT, data: 'wr', nextSibling: null } as MockNode;
    const wc = { nodeType: COMMENT, data: 'wc', nextSibling: null } as MockNode;
    const wi = { nodeType: COMMENT, data: 'wi', nextSibling: null } as MockNode;
    const wcEnd = { nodeType: COMMENT, data: '/wc', nextSibling: null } as MockNode;
    const wrEnd = { nodeType: COMMENT, data: '/wr', nextSibling: null } as MockNode;

    wrStart.nextSibling = wc;
    wc.nextSibling = wi;
    wi.nextSibling = wcEnd;
    wcEnd.nextSibling = wrEnd;

    const { items, end } = collectItemMarkers(wrStart as unknown as Comment);
    assert.equal(items.length, 1, 'should find only <!--wi-->');
    assert.strictEqual(end, wrEnd);
  });
});

// ── nextElement ─────────────────────────────────────────────────

describe('nextElement', () => {
  test('finds the next element after a marker', () => {
    const marker = { nodeType: COMMENT, data: 'wi', nextSibling: null } as MockNode;
    const el = { nodeType: ELEMENT, nextSibling: null } as MockNode;
    marker.nextSibling = el;

    const result = nextElement(marker as unknown as Comment);
    assert.strictEqual(result, el);
  });

  test('skips whitespace text nodes', () => {
    const marker = { nodeType: COMMENT, data: 'wi', nextSibling: null } as MockNode;
    const ws1 = { nodeType: TEXT, data: '  ', nextSibling: null } as MockNode;
    const ws2 = { nodeType: TEXT, data: '\n', nextSibling: null } as MockNode;
    const el = { nodeType: ELEMENT, nextSibling: null } as MockNode;

    marker.nextSibling = ws1;
    ws1.nextSibling = ws2;
    ws2.nextSibling = el;

    const result = nextElement(marker as unknown as Comment);
    assert.strictEqual(result, el);
  });

  test('skips conditional markers to find element', () => {
    // <!--wi--> <!--wc--> <elem/>
    const marker = { nodeType: COMMENT, data: 'wi', nextSibling: null } as MockNode;
    const wc = { nodeType: COMMENT, data: 'wc', nextSibling: null } as MockNode;
    const el = { nodeType: ELEMENT, nextSibling: null } as MockNode;

    marker.nextSibling = wc;
    wc.nextSibling = el;

    const result = nextElement(marker as unknown as Comment);
    assert.strictEqual(result, el, 'should skip <!--wc--> and find element');
  });

  test('returns null when hitting <!--/wr--> before an element', () => {
    const marker = { nodeType: COMMENT, data: 'wi', nextSibling: null } as MockNode;
    const wrEnd = { nodeType: COMMENT, data: '/wr', nextSibling: null } as MockNode;

    marker.nextSibling = wrEnd;

    const result = nextElement(marker as unknown as Comment);
    assert.strictEqual(result, null);
  });

  test('returns null when hitting next <!--wi--> before an element', () => {
    const marker = { nodeType: COMMENT, data: 'wi', nextSibling: null } as MockNode;
    const wi2 = { nodeType: COMMENT, data: 'wi', nextSibling: null } as MockNode;

    marker.nextSibling = wi2;

    const result = nextElement(marker as unknown as Comment);
    assert.strictEqual(result, null);
  });

  test('returns null when no more siblings', () => {
    const marker = { nodeType: COMMENT, data: 'wi', nextSibling: null } as MockNode;

    const result = nextElement(marker as unknown as Comment);
    assert.strictEqual(result, null);
  });

  test('returns null for text-only item with only whitespace', () => {
    const marker = { nodeType: COMMENT, data: 'wi', nextSibling: null } as MockNode;
    const ws = { nodeType: TEXT, data: '  \n  ', nextSibling: null } as MockNode;
    const wrEnd = { nodeType: COMMENT, data: '/wr', nextSibling: null } as MockNode;

    marker.nextSibling = ws;
    ws.nextSibling = wrEnd;

    const result = nextElement(marker as unknown as Comment);
    assert.strictEqual(result, null);
  });
});

// ── findByOrdinal ───────────────────────────────────────────────

describe('findByOrdinal', () => {
  // Helper: build a linked list of children and attach to a mock parent.
  function makeParent(...children: MockNode[]): MockNode {
    for (let i = 0; i < children.length - 1; i++) {
      children[i].nextSibling = children[i + 1];
    }
    return {
      nodeType: ELEMENT,
      firstChild: children[0] ?? null,
    } as unknown as MockNode;
  }

  function el(tag = ''): MockNode {
    return { nodeType: ELEMENT, data: tag, nextSibling: null } as MockNode;
  }
  function text(data = ''): MockNode {
    return { nodeType: TEXT, data, nextSibling: null } as MockNode;
  }
  function comment(data: string): MockNode {
    return { nodeType: COMMENT, data, nextSibling: null } as MockNode;
  }

  test('finds element at ordinal 0 with no structural blocks', () => {
    //  <link> <div>
    const link = el('link');
    const div = el('div');
    const parent = makeParent(link, div);

    assert.strictEqual(findByOrdinal(parent as unknown as Node, ELEMENT, 0), link);
    assert.strictEqual(findByOrdinal(parent as unknown as Node, ELEMENT, 1), div);
  });

  test('skips conditional block content when counting elements', () => {
    // Simulates:  <link> <!--wc--> <p>no results</p> <!--/wc--> <div.grid>
    // Template only has <link> and <div.grid>, so <div.grid> = element ordinal 1.
    const link = el('link');
    const wcStart = comment('wc');
    const p = el('p');
    const wcEnd = comment('/wc');
    const div = el('div');
    const parent = makeParent(link, wcStart, p, wcEnd, div);

    assert.strictEqual(
      findByOrdinal(parent as unknown as Node, ELEMENT, 0), link,
      'ordinal 0 = <link> (before conditional)',
    );
    assert.strictEqual(
      findByOrdinal(parent as unknown as Node, ELEMENT, 1), div,
      'ordinal 1 = <div> (after conditional, skipping <p>)',
    );
  });

  test('skips repeat block content when counting elements', () => {
    // Simulates:  <!--wr--> <card/> <card/> <!--/wr--> <button>
    // Template only has <button>, so <button> = element ordinal 0.
    const wrStart = comment('wr');
    const card1 = el('card');
    const card2 = el('card');
    const wrEnd = comment('/wr');
    const btn = el('button');
    const parent = makeParent(wrStart, card1, card2, wrEnd, btn);

    assert.strictEqual(
      findByOrdinal(parent as unknown as Node, ELEMENT, 0), btn,
      'ordinal 0 = <button> (repeat items skipped)',
    );
  });

  test('skips nested conditional blocks', () => {
    // <!--wc--> <!--wc--> <inner/> <!--/wc--> <outer/> <!--/wc--> <target>
    const wc1 = comment('wc');
    const wc2 = comment('wc');
    const inner = el('inner');
    const wcEnd2 = comment('/wc');
    const outer = el('outer');
    const wcEnd1 = comment('/wc');
    const target = el('target');
    const parent = makeParent(wc1, wc2, inner, wcEnd2, outer, wcEnd1, target);

    assert.strictEqual(
      findByOrdinal(parent as unknown as Node, ELEMENT, 0), target,
      'ordinal 0 = <target> (all nested conditional content skipped)',
    );
  });

  test('skips multiple sequential conditional blocks', () => {
    // <!--wc--> <a/> <!--/wc--> <!--wc--> <b/> <!--/wc--> <target>
    const wc1 = comment('wc');
    const a = el('a');
    const wcEnd1 = comment('/wc');
    const wc2 = comment('wc');
    const b = el('b');
    const wcEnd2 = comment('/wc');
    const target = el('target');
    const parent = makeParent(wc1, a, wcEnd1, wc2, b, wcEnd2, target);

    assert.strictEqual(
      findByOrdinal(parent as unknown as Node, ELEMENT, 0), target,
      'ordinal 0 = <target> (two conditional blocks skipped)',
    );
  });

  test('handles empty conditional blocks (false condition)', () => {
    // <!--wc--> <!--/wc--> <target>
    const wcStart = comment('wc');
    const wcEnd = comment('/wc');
    const target = el('target');
    const parent = makeParent(wcStart, wcEnd, target);

    assert.strictEqual(
      findByOrdinal(parent as unknown as Node, ELEMENT, 0), target,
      'ordinal 0 = <target> (empty conditional skipped)',
    );
  });

  test('skips conditional content for text ordinals too', () => {
    // "hello" <!--wc--> "inside" <!--/wc--> "world"
    const t1 = text('hello');
    const wcStart = comment('wc');
    const t2 = text('inside');
    const wcEnd = comment('/wc');
    const t3 = text('world');
    const parent = makeParent(t1, wcStart, t2, wcEnd, t3);

    assert.strictEqual(
      findByOrdinal(parent as unknown as Node, TEXT, 0), t1,
      'text ordinal 0 = "hello"',
    );
    assert.strictEqual(
      findByOrdinal(parent as unknown as Node, TEXT, 1), t3,
      'text ordinal 1 = "world" (skipping "inside" in conditional)',
    );
  });

  test('skips interleaved conditional and repeat blocks', () => {
    // <!--wc--> <p/> <!--/wc--> <!--wr--> <item/> <!--/wr--> <target>
    const wc = comment('wc');
    const p = el('p');
    const wcEnd = comment('/wc');
    const wr = comment('wr');
    const item = el('item');
    const wrEnd = comment('/wr');
    const target = el('target');
    const parent = makeParent(wc, p, wcEnd, wr, item, wrEnd, target);

    assert.strictEqual(
      findByOrdinal(parent as unknown as Node, ELEMENT, 0), target,
      'ordinal 0 = <target> (both conditional and repeat blocks skipped)',
    );
  });

  test('returns null when ordinal exceeds available children', () => {
    const a = el('a');
    const parent = makeParent(a);

    assert.strictEqual(
      findByOrdinal(parent as unknown as Node, ELEMENT, 5), null,
      'should return null for out-of-range ordinal',
    );
  });

  test('returns null for empty parent', () => {
    const parent = { nodeType: ELEMENT, firstChild: null } as unknown as MockNode;

    assert.strictEqual(
      findByOrdinal(parent as unknown as Node, ELEMENT, 0), null,
      'should return null for parent with no children',
    );
  });
});
