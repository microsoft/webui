// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from 'node:assert/strict';
import { describe, test } from 'node:test';

import { readHydrationEventCounts } from './events.js';

describe('readHydrationEventCounts', () => {
  test('accepts count-based markers that sum to the event array length', () => {
    assert.deepStrictEqual(readHydrationEventCounts(['1', '2', '1'], 4), [1, 2, 1]);
  });

  test('rejects old index-based markers', () => {
    assert.strictEqual(readHydrationEventCounts(['0', '1', '2'], 3), null);
  });

  test('rejects markers that overrun the event array length', () => {
    assert.strictEqual(readHydrationEventCounts(['2', '2'], 3), null);
  });

  test('rejects missing marker values', () => {
    assert.strictEqual(readHydrationEventCounts(['1', null], 1), null);
  });
});
