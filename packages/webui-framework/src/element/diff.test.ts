// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';

import { resolveRepeatValue } from './diff.js';

describe('resolveRepeatValue', () => {
  test('returns item when path equals scope var', () => {
    const item = { name: 'A' };
    assert.equal(resolveRepeatValue('item', item, 'item'), item);
  });

  test('resolves nested property under scope var', () => {
    assert.equal(resolveRepeatValue('item', { name: 'A' }, 'item.name'), 'A');
  });

  test('returns undefined for paths not starting with scope var', () => {
    assert.equal(resolveRepeatValue('item', { name: 'A' }, 'group.name'), undefined);
  });
});

describe('itemKey edge cases', () => {
  // itemKey is not exported, but we can verify the behavior through
  // resolveRepeatValue which it depends on. The key bug was that
  // outer-scope attrs (e.g. data-group="{{group.name}}") produced
  // empty-string keyPaths, causing itemKey to stringify the whole
  // item object as "[object Object]". The fix ensures $repeatMaps
  // never puts empty-string paths into attrMap, but we verify the
  // resolution layer is correct.

  test('resolving outer-scope path returns undefined', () => {
    const item = { value: 'XL', active: true };
    // "group.name" does NOT start with "opt." so it should not resolve
    assert.equal(resolveRepeatValue('opt', item, 'group.name'), undefined);
  });

  test('resolving item-scoped path returns correct value', () => {
    const item = { value: 'XL', active: true };
    assert.equal(resolveRepeatValue('opt', item, 'opt.value'), 'XL');
    assert.equal(resolveRepeatValue('opt', item, 'opt.active'), true);
  });

  test('distinct items produce distinct resolved keys', () => {
    const items = [
      { value: 'XS' },
      { value: 'S' },
      { value: 'M' },
      { value: 'L' },
      { value: 'XL' },
    ];
    const keys = items.map((item) => resolveRepeatValue('opt', item, 'opt.value'));
    const unique = new Set(keys);
    assert.equal(unique.size, items.length, 'all keys must be unique');
  });
});

describe('resolvePath (allocation-free dotted path)', () => {
  test('resolves single-segment path', () => {
    assert.equal(resolveRepeatValue('item', { name: 'A' }, 'item.name'), 'A');
  });

  test('resolves multi-segment path', () => {
    const item = { user: { address: { city: 'Seattle' } } };
    assert.equal(resolveRepeatValue('item', item, 'item.user.address.city'), 'Seattle');
  });

  test('returns undefined for missing intermediate segment', () => {
    assert.equal(resolveRepeatValue('item', { user: null }, 'item.user.name'), undefined);
  });

  test('returns undefined for missing leaf', () => {
    assert.equal(resolveRepeatValue('item', { user: {} }, 'item.user.name'), undefined);
  });

  test('handles numeric-like keys', () => {
    assert.equal(resolveRepeatValue('item', { '0': 'first' }, 'item.0'), 'first');
  });
});
