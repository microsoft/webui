// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { summarize } from './responsiveness-metrics.mjs';

test('fast unreported events retain a bound rather than inventing zero latency', () => {
  const result = summarize([null, null, null, 16, 40], 16);
  assert.deepEqual(result.median, { value: 16, censored: true });
  assert.deepEqual(result.p95, { value: 40, censored: false });
  assert.equal(result.belowThreshold, 3);
  assert.equal(result.count, 5);
});

test('observed threshold values are distinct from censored values', () => {
  assert.deepEqual(summarize([null, 16, 16], 16).median, { value: 16, censored: false });
  assert.deepEqual(summarize([8, 2, 4, 6]).median, { value: 4, censored: false });
});

test('invalid measurements fail instead of looking like successful fast interactions', () => {
  for (const values of [[], [NaN], [-1], [Infinity], [null]]) {
    assert.throws(() => summarize(values));
  }
});
