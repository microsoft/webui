// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { templateHasTopology } from './types.js';
import type { TemplateBlockMeta } from '../template-types.js';

test('empty and text/attribute/event-only metadata have no structural ownership requirement', () => {
  const fixtures: TemplateBlockMeta[] = [
    { h: '' },
    { h: '<span></span>', c: [], r: [], u: [] },
    {
      h: '<button></button><!---->',
      tx: [[[1, 0], [['label']]], [[0, 1], [['suffix']], 7]],
      a: [['title', 0, 'label']], ag: [[1, 0, 1]],
      eg: [['click', [['clicked', [], 1]]]],
    },
  ];
  for (const meta of fixtures) {
    assert.equal(templateHasTopology(meta), false);
    assert.equal(templateHasTopology(meta, 0), false);
  }
});

test('every structural form and raw text binding retains ownership', () => {
  const fixtures: TemplateBlockMeta[] = [
    { h: '', c: [[[() => true, []], 0, [0, 0]]] },
    { h: '', r: [['items', 'item', 0, [0, 0]]] },
    { h: '', u: [[0, [0, 0]]] },
    { h: '', tx: [[[0, 0], [['html']], 1]] },
    { h: '<span></span>', tx: [[[1, 0], [['label']]], [[1, 0, 1], [['html']], 1]] },
  ];
  for (const meta of fixtures) {
    assert.equal(templateHasTopology(meta), true);
    assert.equal(templateHasTopology(meta, meta.tx ? 1 : 0), true);
  }
});

test('SSR reuses eager Shape raw facts without re-reading text metadata', () => {
  const meta: TemplateBlockMeta = { h: '' };
  Object.defineProperty(meta, 'tx', { get(): never { throw new Error('no second raw metadata scan'); } });
  assert.equal(templateHasTopology(meta, 0), false);
  assert.equal(templateHasTopology(meta, 1), true);
});
