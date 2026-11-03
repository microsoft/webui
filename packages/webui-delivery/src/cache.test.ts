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

  test('incomplete and completed streaming response ownership is unchanged', t => {
    let now = 1_000;
    t.mock.method(Date, 'now', () => now);
    const owner = cache();
    const data = response();
    owner.store('/items/1', data, false, true);
    assert.equal(owner.has('/items/1'), true);
    assert.equal(owner.lookup('/items/1'), null);
    const entry = owner.getEntry('/items/1');
    assert.ok(entry);
    entry.complete = true;
    assert.equal(owner.lookup('/items/1'), data);
    now++;
    assert.equal(owner.lookup('/items/1'), null);
    assert.equal(owner.has('/items/1'), true);
    owner.clear();
    assert.equal(owner.has('/items/1'), false);
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
