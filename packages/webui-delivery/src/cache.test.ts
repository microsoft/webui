// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';
import { NavigationCache, type PartialResponse } from './cache.js';

function response(staleTime?: number, tags: string[] = []): PartialResponse {
  return {
    path: '/items/1',
    state: { itemId: '1' },
    templates: {},
    componentStyles: { version: 1, strategy: 'style', resources: {}, closures: {} },
    cacheTags: tags,
    ...(staleTime === undefined ? {} : { cacheControl: { staleTime } }),
  };
}

function cache(staleTime = 0, maxEntries = 50): NavigationCache {
  return new NavigationCache({ staleTime, gcTime: 300_000, maxEntries });
}

describe('completed navigation response ownership', () => {
  test('does not retain an ordinary zero-freshness JSON response', t => {
    let now = 1_000;
    t.mock.method(Date, 'now', () => now);
    const owner = cache();
    owner.store('/items/1', response());

    assert.equal(owner.has('/items/1'), false);
    assert.equal(owner.getEntry('/items/1'), undefined);
    assert.equal(owner.lookup('/items/1'), null, 'zero disables even same-clock reuse');
    now++;
    assert.equal(owner.lookup('/items/1'), null);
    now = 999;
    assert.equal(owner.lookup('/items/1'), null, 'clock rollback cannot restore discarded data');
  });

  test('positive server freshness overrides zero configuration', t => {
    let now = 1_000;
    t.mock.method(Date, 'now', () => now);
    const owner = cache();
    const data = response(100);
    owner.store('/items/1', data);
    now += 100;
    assert.equal(owner.lookup('/items/1'), data);
    now++;
    assert.equal(owner.lookup('/items/1'), null);
  });

  test('explicit server zero overrides positive configuration', () => {
    const owner = cache(60_000);
    owner.store('/items/1', response(0));
    assert.equal(owner.has('/items/1'), false);
    assert.equal(owner.lookup('/items/1'), null);
  });

  test('discarded replacement removes the old response and all tag references', () => {
    const owner = cache();
    owner.store('/items/1', response(60_000, ['old', 'shared']));
    owner.store('/items/2', response(60_000, ['shared']));
    owner.store('/items/1', response(0, ['new']));

    assert.equal(owner.has('/items/1'), false);
    assert.equal(owner.has('/items/2'), true);
    const tags = (owner as unknown as { tagIndex: Map<string, Set<string>> }).tagIndex;
    assert.deepEqual([...tags.keys()], ['shared']);
    assert.deepEqual([...tags.get('shared')!], ['/items/2']);
    owner.invalidateTags(['old', 'new']);
    assert.equal(owner.has('/items/2'), true);
    owner.invalidateTags(['shared']);
    assert.equal(owner.has('/items/2'), false);
    assert.equal(tags.size, 0);
  });

  test('discarded response does not evict an unrelated useful LRU entry', () => {
    const owner = cache(0, 1);
    const useful = response(60_000);
    owner.store('/items/1', useful);
    owner.store('/items/2', response());
    assert.equal(owner.lookup('/items/1'), useful);
    assert.equal(owner.has('/items/2'), false);
  });

  test('positive configuration retains useful responses and LRU behavior', () => {
    const owner = cache(60_000, 2);
    const first = response();
    owner.store('/items/1', first);
    owner.store('/items/2', response());
    assert.equal(owner.lookup('/items/1'), first);
    owner.store('/items/3', response());
    assert.equal(owner.has('/items/1'), true);
    assert.equal(owner.has('/items/2'), false);
    assert.equal(owner.has('/items/3'), true);
  });

  test('preloads retain their inclusive five-second minimum with server zero', t => {
    let now = 1_000;
    t.mock.method(Date, 'now', () => now);
    const owner = cache(60_000);
    const data = response(0);
    owner.store('/items/1', data, true, false);
    now += 5_000;
    assert.equal(owner.lookup('/items/1'), data);
    now++;
    assert.equal(owner.lookup('/items/1'), null);
  });

  test('preloads preserve positive freshness longer than the minimum', t => {
    let now = 1_000;
    t.mock.method(Date, 'now', () => now);
    const owner = cache();
    const data = response(10_000);
    owner.store('/items/1', data, true, false);
    now += 10_000;
    assert.equal(owner.lookup('/items/1'), data);
    now++;
    assert.equal(owner.lookup('/items/1'), null);
  });

  test('ordinary zero-freshness streams release their response and tags on every settlement', t => {
    let now = 1_000;
    t.mock.method(Date, 'now', () => now);
    for (const completed of [true, false]) {
      for (const configured of [0, 60_000]) {
        now = 1_000;
        const owner = cache(configured);
        const data = response(configured === 0 ? undefined : 0, ['stream']);
        owner.store('/items/1', data, false, true);
        assert.equal(owner.has('/items/1'), true);
        assert.equal(owner.lookup('/items/1'), null);
        owner.settleStream('/items/1', data, completed);
        assert.equal(owner.has('/items/1'), false);
        assert.equal(owner.getEntry('/items/1'), undefined);
        assert.equal(owner.lookup('/items/1'), null, 'zero disables even same-clock reuse');
        now = 999;
        assert.equal(owner.lookup('/items/1'), null, 'rollback cannot revive released streams');
        const tags = (owner as unknown as { tagIndex: Map<string, Set<string>> }).tagIndex;
        assert.equal(tags.size, 0);
      }
    }
  });

  test('settling an old same-path stream cannot remove or complete its replacement', () => {
    for (const completed of [true, false]) {
      const owner = cache();
      const old = response(0, ['old']);
      const replacement = response(0, ['replacement']);
      owner.store('/items/1', old, false, true);
      owner.store('/items/1', replacement, false, true);
      owner.settleStream('/items/1', old, completed);
      assert.equal(owner.getEntry('/items/1')?.data, replacement);
      assert.equal(owner.getEntry('/items/1')?.complete, false);
      owner.invalidateTags(['old']);
      assert.equal(owner.has('/items/1'), true);
      owner.settleStream('/items/1', replacement, true);
      assert.equal(owner.has('/items/1'), false);
      owner.settleStream('/items/1', old, true);
      assert.equal(owner.has('/items/1'), false);
    }
  });

  test('stream settlement preserves positive freshness and the inclusive preload minimum', t => {
    let now = 1_000;
    t.mock.method(Date, 'now', () => now);
    for (const preload of [false, true]) {
      now = 1_000;
      const owner = cache();
      const data = response(preload ? 0 : 100);
      owner.store('/items/1', data, preload, true);
      assert.equal(owner.lookup('/items/1'), null);
      owner.settleStream('/items/1', data, true);
      assert.equal(owner.lookup('/items/1'), data);
      now += preload ? 5_000 : 100;
      assert.equal(owner.lookup('/items/1'), data);
      now++;
      assert.equal(owner.lookup('/items/1'), null);
      assert.equal(owner.has('/items/1'), true);
    }
  });

  test('failed positive-freshness and preload streams remain incomplete', () => {
    for (const preload of [false, true]) {
      const owner = cache();
      const data = response(preload ? 0 : 100);
      owner.store('/items/1', data, preload, true);
      owner.settleStream('/items/1', data, false);
      assert.equal(owner.getEntry('/items/1')?.complete, false);
      assert.equal(owner.has('/items/1'), true);
      assert.equal(owner.lookup('/items/1'), null);
    }
  });

  test('stream settlement preserves negative, NaN and infinite lifetime comparisons', t => {
    let now = 1_000;
    t.mock.method(Date, 'now', () => now);
    for (const lifetime of [-1, Number.NaN, Number.POSITIVE_INFINITY]) {
      now = 1_000;
      const owner = cache(lifetime);
      const data = response();
      owner.store('/items/1', data, false, true);
      owner.settleStream('/items/1', data, true);
      assert.equal(owner.has('/items/1'), true);
      assert.equal(owner.lookup('/items/1'), lifetime === -1 ? null : data);
      now = 998;
      assert.equal(owner.lookup('/items/1'), data);
    }
  });

  test('negative, NaN and infinite lifetimes keep their existing comparison semantics', t => {
    let now = 1_000;
    t.mock.method(Date, 'now', () => now);
    for (const lifetime of [-1, Number.NaN, Number.POSITIVE_INFINITY]) {
      now = 1_000;
      const owner = cache(lifetime);
      const data = response();
      owner.store('/items/1', data);
      assert.equal(owner.has('/items/1'), true);
      assert.equal(owner.lookup('/items/1'), lifetime === -1 ? null : data);
      now = 998;
      assert.equal(owner.lookup('/items/1'), data);
    }
  });
});
