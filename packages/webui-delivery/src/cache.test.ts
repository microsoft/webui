// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';
import { NavigationCache, navigationCache, type PartialResponse } from './cache.js';

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

  describe('navigation response byte budget', () => {
    test('rejects invalid budgets and zero retains neither preloads nor streams', () => {
      for (const maxBytes of [-1, NaN, Infinity, 1.5]) {
        assert.throws(() => navigationCache({ maxBytes }), RangeError);
      }
      const owner = navigationCache({ maxBytes: 0, staleTime: 60_000 });
      for (const preload of [false, true]) {
        for (const streaming of [false, true]) {
          owner.store('/items/1', response(), preload, streaming);
          assert.equal(owner.has('/items/1'), false);
        }
      }
    });

    test('evicts by bytes in LRU order and releases accounting on invalidation and clear', () => {
      const sizing = cache(60_000);
      sizing.store('/a', response());
      const size = sizing.getEntry('/a')!.bytes;
      const owner = navigationCache({ staleTime: 60_000, maxBytes: size * 2 });
      const first = response();
      owner.store('/a', first);
      owner.store('/b', response());
      assert.equal(owner.lookup('/a'), first);
      owner.store('/c', response());
      assert.equal(owner.has('/b'), false);
      assert.equal(owner.has('/a'), true);
      owner.invalidate('/a');
      owner.store('/d', response());
      assert.equal(owner.has('/c'), true);
      owner.clear();
      owner.store('/e', response());
      owner.store('/f', response());
      assert.equal(owner.has('/e'), true);
    });

    test('whole-cache invalidation releases all bookkeeping without enumerating keys', t => {
      const owner = navigationCache({ staleTime: 60_000, maxBytes: 4_096 });
      const first = response(60_000, ['first', 'shared']);
      const stream = response(60_000, ['stream', 'shared']);
      const keys = t.mock.method(owner['cache'], 'keys');
      for (const path of [undefined, '']) {
        owner.store('/a', first);
        owner.store('/b', stream, true, true);
        keys.mock.resetCalls();
        owner.invalidate(path);
        assert.equal(keys.mock.callCount(), 0);
        assert.equal(owner.has('/a'), false);
        assert.equal(owner.has('/b'), false);
        assert.equal(owner['tagIndex'].size, 0);
        assert.equal(owner['bytes'], 0);
        const replacement = response(60_000, ['new']);
        owner.store('/b', replacement);
        owner.settleStream('/b', stream, true);
        owner.invalidateTags(['stream']);
        assert.equal(owner.lookup('/b'), replacement);
        owner.invalidate('/b');
      }
    });

    test('an oversized response does not displace unrelated entries or leave stale replacement tags', () => {
      const owner = navigationCache({ staleTime: 60_000, maxBytes: 2_000 });
      owner.store('/a', response(60_000, ['old']));
      owner.store('/b', response());
      const large = response(60_000, ['new']);
      large.state = { text: 'x'.repeat(2_000) };
      owner.store('/a', large, true);
      assert.equal(owner.has('/a'), false);
      assert.equal(owner.has('/b'), true);
      const tags = owner['tagIndex'];
      assert.equal(tags.has('old'), false);
      assert.equal(tags.has('new'), false);
    });

    test('growing stream state is evicted before completion without affecting a replacement', () => {
      const owner = navigationCache({ staleTime: 60_000, maxBytes: 2_000 });
      const first = response();
      owner.store('/a', first, true, true);
      first.state = { text: 'x'.repeat(2_000) };
      owner.refreshStream('/a', first);
      assert.equal(owner.has('/a'), false);
      const replacement = response();
      owner.store('/a', replacement, true, true);
      owner.refreshStream('/a', first);
      owner.settleStream('/a', first, true);
      assert.equal(owner.getEntry('/a')?.data, replacement);
      assert.equal(owner.getEntry('/a')?.complete, false);
      replacement.state = { text: 'y'.repeat(2_000) };
      owner.settleStream('/a', replacement, true);
      assert.equal(owner.has('/a'), false);
    });

    test('lookup does not rescan or serialize cached state and zero freshness never measures it', () => {
      let reads = 0;
      const data = response();
      data.state = { get text() { reads++; return 'value'; } };
      data.state.self = data.state;
      const owner = cache(60_000);
      owner.store('/a', data);
      assert.equal(reads, 1);
      for (let i = 0; i < 10; i++) assert.equal(owner.lookup('/a'), data);
      assert.equal(reads, 1);
      cache().store('/a', data);
      assert.equal(reads, 1);
    });

    test('primitive children need no traversal stack and retain exact admission boundaries', () => {
      const shared = { label: 'shared' };
      const values: unknown[] = Array.from({ length: 256 }, (_, index) => index % 2 ? index : 'row');
      delete values[5];
      values[0] = shared;
      values[1] = shared;
      values[2] = values;
      values[3] = null;
      values[4] = undefined;
      values[6] = false;
      Object.assign(values, { extra: '\u2713\ud83d\ude42' });
      const data = response();
      data.state = { values, shared };
      const owner = cache(60_000);
      const push = Array.prototype.push;
      let primitivePushes = 0;
      try {
        Array.prototype.push = function (...children: unknown[]): number {
          for (const child of children) {
            if (child === null || typeof child !== 'object') primitivePushes++;
          }
          return Reflect.apply(push, this, children) as number;
        };
        owner.store('/mixed', data);
      } finally {
        Array.prototype.push = push;
      }
      const bytes = owner.getEntry('/mixed')!.bytes;
      assert.equal(primitivePushes, 0, 'only object children require later traversal');
      assert.equal(bytes, 3_350, 'sparse, custom, shared and cyclic content keeps the prior charge');
      const exact = navigationCache({ staleTime: 60_000, maxBytes: bytes });
      exact.store('/mixed', data);
      assert.equal(exact.lookup('/mixed'), data);
      assert.equal(exact.getEntry('/mixed')!.bytes, bytes);
      const tooSmall = navigationCache({ staleTime: 60_000, maxBytes: bytes - 1 });
      tooSmall.store('/mixed', data);
      assert.equal(tooSmall.has('/mixed'), false);
    });

    test('multiple state records never revisit immutable metadata or unchanged route states', () => {
      let metadataReads = 0;
      let parentReads = 0;
      const data = response();
      data.templates = { get template() { metadataReads++; return 'x'.repeat(100_000); } };
      data.chain = [
        { component: 'parent-view', path: '/', params: {}, state: {
          get retained() { parentReads++; return 'parent'; },
        } },
        { component: 'child-view', path: '/child', params: {} },
      ];
      const owner = navigationCache({ staleTime: 60_000 });
      owner.store('/child', data, false, true);
      const baseBytes = owner.getEntry('/child')!.bytes;
      for (let i = 0; i < 50; i++) {
        const state = { value: 'new'.repeat(i + 1) };
        data.chain[1].state = state;
        owner.refreshStream('/child', data, [null, state]);
      }
      assert.equal(metadataReads, 1);
      assert.equal(parentReads, 1);
      assert.ok(owner.getEntry('/child')!.bytes > baseBytes);
      owner.settleStream('/child', data, true, true);
      assert.equal(metadataReads, 1, 'settlement of already accounted records does not rescan');
      assert.equal(parentReads, 1);
      assert.equal(owner.getEntry('/child')!.stateBytes, undefined);
      assert.equal(owner.lookup('/child'), data);
    });

    test('buffered state-bearing route chains are accounted with linear owner lookups', () => {
      const count = 512;
      let entryReads = 0;
      const chain = new Proxy(Array.from({ length: count }, (_, index) => ({
        component: 'route-view', path: '/' + index, params: {},
        state: { value: index, nested: { state: 'payload' } },
      })), {
        get(target, key, receiver) {
          if (typeof key === 'string' && Number.isInteger(Number(key))) entryReads++;
          return Reflect.get(target, key, receiver);
        },
      });
      const data = response(60_000);
      data.chain = chain;
      const owner = navigationCache();
      owner.store('/chain', data, true, true);
      assert.ok(entryReads <= count * 4, `expected linear chain access, got ${entryReads} reads`);
      const entry = owner.getEntry('/chain')!;
      assert.equal(entry.stateBytes?.length, count);
      const bytes = entry.bytes;
      entryReads = 0;
      owner.refreshStream('/chain', data);
      assert.ok(entryReads <= count * 4, `expected linear recount, got ${entryReads} reads`);
      assert.equal(entry.bytes, bytes);
      const exact = navigationCache({ maxBytes: bytes });
      exact.store('/chain', data, true, true);
      assert.equal(exact.getEntry('/chain')!.bytes, bytes);
      const undersized = navigationCache({ maxBytes: bytes - 1 });
      undersized.store('/chain', data, true, true);
      assert.equal(undersized.has('/chain'), false);
    });

    test('incremental replacements release old bytes and skip null state slots', () => {
      const owner = navigationCache({ staleTime: 60_000, maxBytes: 2_000 });
      const data = response();
      data.chain = [{ component: 'test-view', path: '/', params: {}, state: { text: 'a'.repeat(300) } }];
      owner.store('/', data, true, true);
      const full = owner.getEntry('/')!.bytes;
      const small = { text: 'b' };
      data.chain[0].state = small;
      owner.refreshStream('/', data, [small]);
      assert.equal(owner.getEntry('/')!.bytes, full - 598);
      owner.refreshStream('/', data, [null]);
      assert.equal(owner.getEntry('/')!.bytes, full - 598);
      const large = { text: 'c'.repeat(2_000) };
      data.chain[0].state = large;
      owner.refreshStream('/', data, [large]);
      assert.equal(owner.has('/'), false);
    });
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
