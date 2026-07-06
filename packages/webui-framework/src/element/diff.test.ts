// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';

import { dotWalk } from './diff.js';

describe('dotWalk (allocation-free dotted path)', () => {
  test('resolves a single-segment path', () => {
    assert.equal(dotWalk({ name: 'A' }, 'name', 0), 'A');
  });

  test('resolves a multi-segment nested path', () => {
    const item = { user: { address: { city: 'Seattle' } } };
    assert.equal(dotWalk(item, 'user.address.city', 0), 'Seattle');
  });

  test('returns undefined for a missing intermediate segment', () => {
    assert.equal(dotWalk({ user: null }, 'user.name', 0), undefined);
  });

  test('returns undefined for a missing leaf', () => {
    assert.equal(dotWalk({ user: {} }, 'user.name', 0), undefined);
  });

  test('handles numeric-like keys', () => {
    assert.equal(dotWalk({ '0': 'first' }, '0', 0), 'first');
  });

  test('walks from a start offset (skips the scope-var prefix)', () => {
    // Simulates resolving `item.name` against the `item` scope variable.
    assert.equal(dotWalk({ name: 'A' }, 'item.name', 5), 'A');
  });
});
