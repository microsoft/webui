// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { describe, test } from 'node:test';
import assert from 'node:assert/strict';

import { coerceSeedValue, seedObservablePath } from './seed.js';

describe('coerceSeedValue', () => {
  test('returns string value unchanged when current is undefined', () => {
    assert.strictEqual(coerceSeedValue(undefined, 'hello'), 'hello');
  });

  test('returns string value unchanged when current is a string', () => {
    assert.strictEqual(coerceSeedValue('default', 'hello'), 'hello');
  });

  test('coerces string "42" to number when current is a number', () => {
    assert.strictEqual(coerceSeedValue(0, '42'), 42);
  });

  test('coerces string "3.14" to float when current is a number', () => {
    assert.strictEqual(coerceSeedValue(0, '3.14'), 3.14);
  });

  test('returns string as-is when it is not a valid number', () => {
    assert.strictEqual(coerceSeedValue(0, 'not-a-number'), 'not-a-number');
  });

  test('coerces string "true" to boolean when current is a boolean', () => {
    assert.strictEqual(coerceSeedValue(false, 'true'), true);
  });

  test('coerces string "false" to boolean false when current is a boolean', () => {
    assert.strictEqual(coerceSeedValue(true, 'false'), false);
  });

  test('passes through boolean value when current is a boolean', () => {
    assert.strictEqual(coerceSeedValue(false, true), true);
  });

  test('passes through number value when current is a number', () => {
    assert.strictEqual(coerceSeedValue(0, 7), 7);
  });

  test('passes through array values without coercion', () => {
    const arr = [1, 2, 3];
    assert.deepStrictEqual(coerceSeedValue(undefined, arr), arr);
  });
});

describe('seedObservablePath', () => {
  test('seeds a flat observable property', () => {
    const target: Record<string, unknown> = { count: 0 };
    const names = new Set(['count']);
    const seeded = new Set<string>();

    seedObservablePath(target, 'count', '42', names, seeded);

    assert.strictEqual(target.count, 42); // coerced from string
    assert.ok(seeded.has('count'));
  });

  test('ignores paths not in observableNames', () => {
    const target: Record<string, unknown> = { count: 0 };
    const names = new Set(['count']);

    seedObservablePath(target, 'title', 'hello', names);

    assert.strictEqual(target.title, undefined);
  });

  test('ignores empty paths', () => {
    const target: Record<string, unknown> = { count: 0 };
    const names = new Set(['count']);
    const seeded = new Set<string>();

    seedObservablePath(target, '', 'hello', names, seeded);

    assert.strictEqual(seeded.size, 0);
  });

  test('seeds nested path item.title', () => {
    const target: Record<string, unknown> = { item: {} };
    const names = new Set(['item']);
    const seeded = new Set<string>();

    seedObservablePath(target, 'item.title', 'Hello', names, seeded);

    assert.deepStrictEqual(target.item, { title: 'Hello' });
    assert.ok(seeded.has('item.title'));
  });

  test('seeds deeply nested path item.meta.count', () => {
    const target: Record<string, unknown> = { item: {} };
    const names = new Set(['item']);

    seedObservablePath(target, 'item.meta.count', '5', names);

    assert.deepStrictEqual(target.item, { meta: { count: '5' } });
  });

  test('creates intermediate objects for nested paths', () => {
    const target: Record<string, unknown> = {};
    const names = new Set(['data']);

    seedObservablePath(target, 'data.nested.value', 'x', names);

    assert.deepStrictEqual(target.data, { nested: { value: 'x' } });
  });

  test('coerces nested values against existing defaults', () => {
    const inner = { active: false };
    const target: Record<string, unknown> = { config: inner };
    const names = new Set(['config']);

    seedObservablePath(target, 'config.active', 'true', names);

    assert.strictEqual((target.config as Record<string, unknown>).active, true);
  });

  test('seeds array values without coercion', () => {
    const target: Record<string, unknown> = { items: [] };
    const names = new Set(['items']);

    seedObservablePath(target, 'items', [1, 2, 3], names);

    assert.deepStrictEqual(target.items, [1, 2, 3]);
  });

  test('replaces null root with object for nested paths', () => {
    const target: Record<string, unknown> = { data: null };
    const names = new Set(['data']);

    seedObservablePath(target, 'data.key', 'val', names);

    assert.deepStrictEqual(target.data, { key: 'val' });
  });

  test('replaces array root with object for nested paths', () => {
    const target: Record<string, unknown> = { data: [1, 2] };
    const names = new Set(['data']);

    seedObservablePath(target, 'data.key', 'val', names);

    assert.deepStrictEqual(target.data, { key: 'val' });
  });

  test('works without seededPaths parameter', () => {
    const target: Record<string, unknown> = { count: 0 };
    const names = new Set(['count']);

    seedObservablePath(target, 'count', '7', names);

    assert.strictEqual(target.count, 7);
  });
});
