// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';
import { collectItemMarkers, nextElement } from './markers.js';

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
