// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { isRawStartMarker, rawMarker } from './markers.js';

// SSR indexing/range assertions live in hydration.test.ts and exercise the
// same engine as production, rather than the removed range-skipping helpers.
test('raw range labels pair exact decimal identifiers', () => {
  for (const index of [0, 1, 9, 10, 100000]) {
    assert.equal(rawMarker(index), `w${index}`);
    assert.equal(rawMarker(index, true), `/w${index}`);
    assert.equal(isRawStartMarker(rawMarker(index)), true);
    assert.equal(isRawStartMarker(rawMarker(index, true)), false);
  }
});

test('raw marker recognition excludes structural and authored comments', () => {
  for (const label of ['', 'w', 'wr', 'wc', 'wi', 'wf', 'wf:1', '/wh', 'w-1', 'w1x', 'w 1']) {
    assert.equal(isRawStartMarker(label), false, label);
  }
});
