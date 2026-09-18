// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { beforeEach, describe, test } from 'node:test';
import {
  clearFragmentInputSources,
  forgetFragmentInput,
  registerFragmentSources,
  registerFragmentSourceRefs,
  retainFragmentInput,
  retainedFragmentInput,
  adoptFragmentInput,
  releaseFragmentInput,
  fragmentInput,
  fragmentSourceId,
  createFragmentSourceCapture,
} from './fragment-inputs.js';
import type { FragmentSourceNode } from './streaming-protocol.js';
import { scopeSourceRoot } from './element/types.js';

describe('streamed fragment input provenance', () => {
  beforeEach(() => clearFragmentInputSources(true));

  test('resolves additive source, path, item and UTF-8 length nodes without copying', () => {
    const first = {} as Element;
    const second = {} as Element;
    const root = { children: [{ label: 'é😀' }] };
    registerFragmentSources([[0, 0, root], [1, 1, 0, 'children'], [2, 2, 1, 0]]);
    registerFragmentSourceRefs(first, [2]);
    registerFragmentSources([[3, 1, 2, 'label.length']]);
    registerFragmentSourceRefs(second, [3, 2]);
    assert.equal(fragmentInput(first, 2), root.children[0]);
    assert.equal(fragmentInput(second, 3), 6);
    assert.equal(fragmentInput(second, 2), root.children[0]);
    assert.equal(fragmentInput(first, 2), root.children[0], 'shared IDs remain available to sibling calls');
  });

  test('terminal releases sources while lazy and undefined hosts retain their inputs', () => {
    const host = {} as Element;
    registerFragmentSources([[0, 0, null], [1, 0, false], [2, 0, 0]]);
    registerFragmentSourceRefs(host, [0, 1, 2]);
    clearFragmentInputSources();
    assert.equal(fragmentInput(host, 0), null);
    assert.equal(fragmentInput(host, 1), false);
    assert.equal(fragmentInput(host, 2), 0);
    assert.throws(
      () => registerFragmentSourceRefs({} as Element, [2]),
      /unknown source/,
    );
  });

  test('failure abandons pending inputs and rejects unknown input references', () => {
    const host = {} as Element;
    registerFragmentSources([[0, 0, 'captured']]);
    assert.throws(() => registerFragmentSourceRefs(host, [4]), /unknown source/);
    // An open response owns the whole identifier space, so a marker it cannot
    // resolve is real server/client skew rather than an expired reference.
    assert.throws(() => fragmentInput(host, 4), /unknown source/);
    registerFragmentSourceRefs(host, [0]);
    clearFragmentInputSources(true);
    assert.throws(
      () => fragmentInput(host, 0),
      /unknown source 0 after the response closed/,
      'an abandoned response drops its values; it does not license resolving them from owner state',
    );
  });

  test('rejects forward, duplicate and missing projection sources', () => {
    const host = {} as Element;
    assert.throws(() => registerFragmentSources([[1, 1, 0, 'child']]), /unknown source/);
    registerFragmentSources([[0, 0, []]]);
    assert.throws(() => registerFragmentSources([[0, 0, 'replacement']]), /duplicate source/);
    assert.throws(() => registerFragmentSources([[2, 2, 0, 0]]), /missing value/);
    registerFragmentSourceRefs(host, [0, 0]);
    assert.deepEqual(fragmentInput(host, 0), []);
  });

  test('projection paths carry property names only, never array indices', () => {
    const host = {} as Element;
    registerFragmentSources([[0, 0, { rows: [{ label: 'leaf' }], name: 'é' }]]);
    // An element of a collection is addressed by an item tuple, so a numeric
    // segment inside a path is not part of the grammar and must not resolve.
    assert.throws(() => registerFragmentSources([[1, 1, 0, 'rows.0']]), /missing value/);
    assert.throws(() => registerFragmentSources([[2, 1, 0, 'rows.0.label']]), /missing value/);
    // `length` terminates a path: a collection exposes its count and a string
    // its UTF-8 byte count, and neither one can be walked any further.
    assert.throws(() => registerFragmentSources([[3, 1, 0, 'rows.length.more']]), /missing value/);
    assert.throws(() => registerFragmentSources([[4, 1, 0, 'name.bytes']]), /missing value/);
    // An item tuple only indexes arrays; an object is not positionally addressed.
    assert.throws(() => registerFragmentSources([[7, 2, 0, 0]]), /missing value/);
    registerFragmentSources([[5, 1, 0, 'rows.length'], [6, 1, 0, 'name.length']]);
    registerFragmentSourceRefs(host, [5, 6]);
    assert.equal(fragmentInput(host, 5), 1);
    assert.equal(fragmentInput(host, 6), 2);
  });

  test('carries every JSON value through projections and items, and only rejects absent ones', () => {
    const host = {} as Element;
    registerFragmentSources([[0, 0, {
      nil: null,
      off: false,
      zero: 0,
      blank: '',
      empty: [],
      items: [null, false, 0, ''],
    }]]);
    registerFragmentSources([
      [1, 1, 0, 'nil'],
      [2, 1, 0, 'off'],
      [3, 1, 0, 'zero'],
      [4, 1, 0, 'blank'],
      [5, 1, 0, 'empty'],
      [6, 1, 0, 'items'],
      [7, 2, 6, 0],
      [8, 2, 6, 1],
      [9, 2, 6, 2],
      [10, 2, 6, 3],
      // A falsy count is a value like any other and must survive registration.
      [11, 1, 0, 'empty.length'],
      [12, 1, 0, 'blank.length'],
    ]);
    // Only an absent member is missing; walking *through* a null is absent too.
    assert.throws(() => registerFragmentSources([[13, 1, 0, 'nil.child']]), /missing value/);
    assert.throws(() => registerFragmentSources([[14, 1, 0, 'absent']]), /missing value/);
    assert.throws(() => registerFragmentSources([[15, 2, 5, 0]]), /missing value/);
    registerFragmentSourceRefs(host, [1, 2, 3, 4, 7, 8, 9, 10, 11, 12]);
    clearFragmentInputSources();
    assert.equal(fragmentInput(host, 1), null);
    assert.equal(fragmentInput(host, 2), false);
    assert.equal(fragmentInput(host, 3), 0);
    assert.equal(fragmentInput(host, 4), '');
    assert.equal(fragmentInput(host, 7), null);
    assert.equal(fragmentInput(host, 8), false);
    assert.equal(fragmentInput(host, 9), 0);
    assert.equal(fragmentInput(host, 10), '');
    assert.equal(fragmentInput(host, 11), 0);
    assert.equal(fragmentInput(host, 12), 0);
  });

  test('reserved inputs are released by their last adopter, not by host lifetime', () => {
    const host = {} as Element;
    registerFragmentSources([[0, 0, { label: 'OLD' }], [1, 0, 'dormant']]);
    registerFragmentSourceRefs(host, [0, 1]);
    clearFragmentInputSources();
    // Two invocations in the same host resolve through one deduplicated id.
    const first = adoptFragmentInput(host, 0);
    const second = adoptFragmentInput(host, 0);
    assert.equal(first, second);
    releaseFragmentInput(host, 0);
    assert.deepEqual(
      fragmentInput(host, 0),
      { label: 'OLD' },
      'a sibling invocation still resolves through the reservation',
    );
    releaseFragmentInput(host, 0);
    assert.throws(
      () => fragmentInput(host, 0),
      /unknown source 0 after the response closed/,
      'the last abandonment drops the captured value while the host stays alive',
    );
    // Abandoning more claims than were made cannot release anything, and a
    // reservation no invocation ever adopted belongs to a range that has not
    // had its chance to activate yet.
    releaseFragmentInput(host, 0);
    releaseFragmentInput(host, 1);
    assert.equal(fragmentInput(host, 1), 'dormant');
  });

  test('only a surrendered capture identity may fall back to the caller path', () => {
    const host = {} as Element;
    const kept = {} as Comment;
    const surrendered = {} as Comment;
    registerFragmentSources([[0, 0, { label: 'OLD' }]]);
    clearFragmentInputSources();
    // An explicit input write or a removed invocation gives the identity up, so
    // the marker legitimately resolves nothing and its caller path takes over.
    forgetFragmentInput(surrendered);
    assert.equal(fragmentInput(host, 0, surrendered), undefined);
    // An invocation that never surrendered anything still claims a captured
    // value, so a closed table is a retention defect and must not be answered
    // with whatever the owner holds now.
    assert.throws(
      () => fragmentInput(host, 0, kept),
      /unknown source 0 after the response closed/,
    );
  });

  test('seeded adopters also release the host reservation after rebinding', () => {
    const host = {} as Element;
    const anchor = { data: 'wf:0' } as Comment;
    const value = { label: 'OLD' };
    registerFragmentSources([[0, 0, value]]);
    const capture = createFragmentSourceCapture();
    assert.ok(capture);
    capture.visit(anchor);
    registerFragmentSourceRefs(host, [0]);
    clearFragmentInputSources();
    assert.equal(adoptFragmentInput(host, 0, anchor), value);
    forgetFragmentInput(anchor);
    releaseFragmentInput(host, 0);
    assert.throws(() => fragmentInput(host, 0), /unknown source/);
  });

  test('surrendered identities ignore still-open sources and sibling reservations', () => {
    const host = {} as Element;
    const anchor = { data: 'wf:0' } as Comment;
    registerFragmentSources([[0, 0, { label: 'OLD' }]]);
    registerFragmentSourceRefs(host, [0]);
    const capture = createFragmentSourceCapture();
    assert.ok(capture);
    capture.visit(anchor);
    forgetFragmentInput(anchor);
    assert.equal(fragmentInput(host, 0, anchor), undefined);
    assert.equal(adoptFragmentInput(host, 0, anchor), undefined);
    capture.visit(anchor);
    assert.equal(fragmentInput(host, 0, anchor), undefined);
    clearFragmentInputSources();
    assert.equal(adoptFragmentInput(host, 0, anchor), undefined);
  });

  test('stream failure does not restore a surrendered capture identity', () => {
    const host = {} as Element;
    const anchor = {} as Comment;
    forgetFragmentInput(anchor);
    clearFragmentInputSources(true);
    assert.equal(fragmentInput(host, 0, anchor), undefined);
  });

  test('resolves deep projection DAGs iteratively with one lookup per node', () => {
    const host = {} as Element;
    const nodes: FragmentSourceNode[] = [[0, 0, { child: null }]];
    const root = nodes[0][2] as { child: unknown };
    let value = root;
    let reads = 0;
    for (let i = 1; i <= 20_000; i++) {
      const next = { child: null };
      Object.defineProperty(value, 'child', { get: () => { reads++; return next; } });
      value = next;
      nodes.push([i, 1, i - 1, 'child']);
    }
    registerFragmentSources(nodes);
    registerFragmentSourceRefs(host, [20_000]);
    clearFragmentInputSources();
    assert.equal(fragmentInput(host, 20_000), value);
    assert.equal(reads, 20_000);
  });

  test('retains reconnect scopes weakly and releases them after explicit input updates', () => {
    const anchor = {} as Comment;
    const scope = { name: 'node', value: { label: 'old' } };
    retainFragmentInput(anchor, scope, 7);
    assert.equal(retainedFragmentInput(anchor)?.scope, scope);
    assert.equal(retainedFragmentInput(anchor)?.version, 7);
    forgetFragmentInput(anchor);
    assert.equal(retainedFragmentInput(anchor), undefined);
  });

  test('scope provenance follows isolated aliases and caller loop roots, not values', () => {
    const alias = { name: 'node', value: null, sourceRoot: 'tree', isAlias: true as const };
    const loop = { name: 'item', value: null, sourceRoot: 'tree', parent: alias };
    assert.equal(scopeSourceRoot('item', loop), 'tree');
    assert.deepEqual(scopeSourceRoot('item.children', loop), ['tree', 'item']);
    assert.equal(scopeSourceRoot('node.children', loop), 'tree');
    assert.equal(scopeSourceRoot('title', loop), 'title');
    assert.equal(scopeSourceRoot('node.children'), 'node');
    const roots = ['tree', 'item'];
    assert.equal(scopeSourceRoot('node.children', { ...alias, sourceRoot: roots }), roots);
    assert.equal(scopeSourceRoot('item.children', { ...loop, sourceRoot: roots }), roots);
    assert.deepEqual(
      scopeSourceRoot('child.children', { ...loop, name: 'child', sourceRoot: roots }),
      ['tree', 'item', 'child'],
    );
  });

  test('decodes bounded decimal source markers and rejects malformed identifiers', () => {
    assert.equal(fragmentSourceId('wf:0'), 0);
    assert.equal(fragmentSourceId('wf:4294967295'), 0xffffffff);
    for (const data of ['wf:', 'wf:-1', 'wf:1x', 'wf:1.2', 'wf:4294967296']) {
      assert.throws(() => fragmentSourceId(data), /source ID/);
    }
  });

  test('the existing streaming walk seeds non-spanning deferred markers but excludes raw HTML', () => {
    assert.equal(createFragmentSourceCapture(), undefined);
    const parent = {} as Node, nested = {} as Node, host = {} as Element;
    const comment = (data: string, parentNode = parent) => ({ data, parentNode }) as Comment;
    const old = { label: 'OLD' };
    registerFragmentSources([[0, 0, old]]);
    const capture = createFragmentSourceCapture()!;
    const raw = comment('wf:999'), childRaw = comment('wf:998', nested);
    const real = comment('wf:0');
    for (const node of [comment('w0'), raw, childRaw, comment('w0'), comment('/w0'), comment('/w0'), real]) {
      capture.visit(node);
    }
    assert.throws(() => fragmentInput(host, 999, raw), /unknown source/, 'raw HTML contents are opaque');
    clearFragmentInputSources();
    assert.equal(fragmentInput(host, 0, real), old);
    assert.throws(
      () => fragmentInput(host, 0, real),
      /unknown source 0 after the response closed/,
      'the seed belongs to the invocation that adopted it, and losing it is a defect to surface',
    );
    assert.throws(() => fragmentInput(host, 998, childRaw), /unknown source 998/);
  });
});
