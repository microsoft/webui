// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';

import {
  createRepeatKeyState,
  clearRepeatKeyState,
  dotWalk,
  seedHydratedRepeatKeys,
  syncRepeat,
} from './diff.js';
import type {
  RepeatBinding,
  RepeatHost,
  ScopeFrame,
  TemplateInstance,
} from './types.js';

test('repeat identity cleanup reuses its arrays and map without changing key policy', () => {
  const state = createRepeatKeyState('id');
  const keys = state.keys;
  const next = state.nextKeys;
  const map = state.map;
  keys.push('old');
  next.push('next');
  map.set('old', null);
  state.warned = true;
  clearRepeatKeyState(state);
  assert.equal(state.keys, keys);
  assert.equal(state.nextKeys, next);
  assert.equal(state.map, map);
  assert.equal(keys.length, 0);
  assert.equal(next.length, 0);
  assert.equal(map.size, 0);
  assert.equal(state.path, 'id');
  assert.equal(state.warned, true);
});

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

function emptyInstance(): TemplateInstance {
  return {
    container: null,
    nodes: [],
    texts: [],
    attrs: [],
    conds: [],
    repeats: [],
  };
}

function itemInstance(value: unknown): TemplateInstance {
  const instance = emptyInstance();
  instance.scope = { name: 'item', value, known: true };
  return instance;
}

function repeatHost(
  items: unknown[],
  created: TemplateInstance[],
  updated: TemplateInstance[],
  removed: TemplateInstance[],
  inserted: TemplateInstance[],
): RepeatHost {
  return {
    $resolveValue: () => items,
    $hasStateRoot: () => true,
    $createBlockInstance: (
      _blockIndex: number,
      scope?: ScopeFrame,
      parent?: TemplateInstance,
      _container?: ParentNode & Node,
    ) => {
      const instance = emptyInstance();
      instance.scope = scope;
      instance.parent = parent;
      created.push(instance);
      return instance;
    },
    $updateInstance: (instance: TemplateInstance) => {
      updated.push(instance);
    },
    $removeInstance: (instance: TemplateInstance) => {
      removed.push(instance);
    },
    $changeStructure: () => {},
    $insertInstanceAfter: (
      cursor: Node | null,
      _container: ParentNode & Node,
      instance: TemplateInstance,
    ) => {
      inserted.push(instance);
      return cursor;
    },
  };
}

function repeatBinding(
  items: unknown[],
  instances: TemplateInstance[],
): RepeatBinding {
  return {
    collection: 'items',
    itemVar: 'item',
    blockIndex: 0,
    container: { childNodes: [] } as unknown as ParentNode & Node,
    start: null,
    owner: emptyInstance(),
    instances: items.map((_value, index) => instances[index]),
    synced: true,
  };
}

function keyedRepeatBinding(
  items: unknown[],
  instances: TemplateInstance[],
  path: string,
  keys: Array<string | number>,
): RepeatBinding {
  const repeat = repeatBinding(items, instances);
  const keyState = createRepeatKeyState(path);
  keyState.keys = keys;
  repeat.keyState = keyState;
  return repeat;
}

describe('syncRepeat', () => {
  test('stable keyed rows re-read mutated values without uniqueness maps or placement checks', t => {
    const items = [{ id: 'a', value: 1 }, { id: 'b', value: 2 }];
    const instances = items.map(itemInstance);
    const repeat = keyedRepeatBinding(items, instances, 'id', ['a', 'b']);
    repeat.fixed = true;
    const updated: TemplateInstance[] = [];
    const inserted: TemplateInstance[] = [];
    const host = repeatHost(items, [], updated, [], inserted);
    const writes = t.mock.method(repeat.keyState!.map, 'set');
    items[0].value = 3;
    syncRepeat(host, repeat);
    assert.deepEqual(updated, instances);
    assert.equal(instances[0].scope?.value, items[0]);
    assert.equal(writes.mock.callCount(), 0);
    assert.deepEqual(inserted, []);

    items.push({ id: 'c', value: 4 });
    syncRepeat(host, repeat);
    assert.equal(inserted.length, 1, 'only a new tail needs placement');
    items[1].id = 'a';
    const warn = t.mock.method(console, 'warn', () => {});
    syncRepeat(host, repeat);
    assert.equal(warn.mock.callCount(), 1);
    assert.deepEqual(repeat.keyState!.keys, [], 'in-place duplicate keys lose identity');
    items[1].id = 'b';
    syncRepeat(host, repeat);
    assert.deepEqual(repeat.keyState!.keys, ['a', 'b', 'c']);
  });

  test('reuses entries by position when item attributes are duplicated', () => {
    const oldItems = [
      { label: 'one', className: 'same' },
      { label: 'two', className: 'same' },
      { label: 'three', className: 'same' },
    ];
    const newItems = [
      { label: 'updated-one', className: 'same' },
      { label: 'updated-two', className: 'same' },
    ];
    const oldInstances = oldItems.map(itemInstance);
    const created: TemplateInstance[] = [];
    const updated: TemplateInstance[] = [];
    const removed: TemplateInstance[] = [];
    const inserted: TemplateInstance[] = [];
    const repeat = repeatBinding(oldItems, oldInstances);
    const host = repeatHost(
      newItems,
      created,
      updated,
      removed,
      inserted,
    );

    syncRepeat(host, repeat);

    assert.deepEqual(created, []);
    assert.deepEqual(updated, oldInstances.slice(0, 2));
    assert.deepEqual(removed, [oldInstances[2]]);
    assert.deepEqual(inserted, oldInstances.slice(0, 2));
    assert.equal(repeat.instances.length, 2);
    assert.equal(repeat.instances[0], oldInstances[0]);
    assert.equal(repeat.instances[1], oldInstances[1]);
    assert.equal(oldInstances[0].scope?.value, newItems[0]);
    assert.equal(oldInstances[1].scope?.value, newItems[1]);
  });

  test('appends only the new positional tail', () => {
    const firstItem = { label: 'one' };
    const firstInstance = itemInstance(firstItem);
    const newItems = [firstItem, { label: 'two' }, { label: 'three' }];
    const created: TemplateInstance[] = [];
    const updated: TemplateInstance[] = [];
    const removed: TemplateInstance[] = [];
    const inserted: TemplateInstance[] = [];
    const repeat = repeatBinding([firstItem], [firstInstance]);
    const host = repeatHost(
      newItems,
      created,
      updated,
      removed,
      inserted,
    );

    syncRepeat(host, repeat);

    assert.deepEqual(updated, [firstInstance]);
    assert.deepEqual(removed, []);
    assert.equal(repeat.instances.length, 3);
    assert.equal(repeat.instances[0], firstInstance);
    assert.deepEqual(repeat.instances.slice(1), created);
    assert.deepEqual(inserted, repeat.instances);
    assert.equal(created[0].scope?.value, newItems[1]);
    assert.equal(created[1].scope?.value, newItems[2]);
  });

  test('moves established explicit keys with their instances', () => {
    const oldItems = [{ id: 1 }, { id: 2 }, { id: 3 }];
    const newItems = [{ id: 3 }, { id: 1 }, { id: 2 }];
    const oldInstances = oldItems.map(itemInstance);
    const created: TemplateInstance[] = [];
    const updated: TemplateInstance[] = [];
    const removed: TemplateInstance[] = [];
    const inserted: TemplateInstance[] = [];
    const repeat = keyedRepeatBinding(oldItems, oldInstances, 'id', [1, 2, 3]);
    const host = repeatHost(
      newItems,
      created,
      updated,
      removed,
      inserted,
    );

    syncRepeat(host, repeat);

    assert.deepEqual(repeat.instances, [
      oldInstances[2],
      oldInstances[0],
      oldInstances[1],
    ]);
    assert.deepEqual(inserted, repeat.instances);
    assert.deepEqual(updated, repeat.instances);
    assert.deepEqual(created, []);
    assert.deepEqual(removed, []);
    assert.deepEqual(repeat.keyState?.keys, [3, 1, 2]);
    assert.equal(oldInstances[2].scope?.value, newItems[0]);
  });

  test('uses the positional fast path when explicit key order is stable', () => {
    const oldItems = [{ id: 'a', label: 'old' }, { id: 'b', label: 'old' }];
    const newItems = [{ id: 'a', label: 'new' }, { id: 'b', label: 'new' }];
    const oldInstances = oldItems.map(itemInstance);
    const created: TemplateInstance[] = [];
    const updated: TemplateInstance[] = [];
    const removed: TemplateInstance[] = [];
    const inserted: TemplateInstance[] = [];
    const repeat = keyedRepeatBinding(
      oldItems,
      oldInstances,
      'id',
      ['a', 'b'],
    );
    const instances = repeat.instances;
    const host = repeatHost(
      newItems,
      created,
      updated,
      removed,
      inserted,
    );

    syncRepeat(host, repeat);

    assert.deepEqual(repeat.instances, oldInstances);
    assert.deepEqual(updated, oldInstances);
    assert.deepEqual(created, []);
    assert.deepEqual(removed, []);
    assert.equal(repeat.instances, instances);
  });

  test('reuses surviving keys and replaces removed keys', () => {
    const oldItems = [{ id: 1 }, { id: 2 }];
    const newItems = [{ id: 1 }, { id: 3 }];
    const oldInstances = oldItems.map(itemInstance);
    const created: TemplateInstance[] = [];
    const updated: TemplateInstance[] = [];
    const removed: TemplateInstance[] = [];
    const inserted: TemplateInstance[] = [];
    const repeat = keyedRepeatBinding(oldItems, oldInstances, 'id', [1, 2]);
    const host = repeatHost(
      newItems,
      created,
      updated,
      removed,
      inserted,
    );

    syncRepeat(host, repeat);

    assert.equal(repeat.instances[0], oldInstances[0]);
    assert.equal(repeat.instances[1], created[0]);
    assert.deepEqual(updated, [oldInstances[0]]);
    assert.deepEqual(removed, [oldInstances[1]]);
    assert.deepEqual(repeat.keyState?.keys, [1, 3]);
  });

  test('falls back safely on duplicate keys and re-establishes identity', () => {
    const oldItems = [{ id: 1 }, { id: 2 }];
    const duplicateItems = [{ id: 3 }, { id: 3 }];
    const oldInstances = oldItems.map(itemInstance);
    const created: TemplateInstance[] = [];
    const updated: TemplateInstance[] = [];
    const removed: TemplateInstance[] = [];
    const inserted: TemplateInstance[] = [];
    const repeat = keyedRepeatBinding(oldItems, oldInstances, 'id', [1, 2]);
    let currentItems = duplicateItems;
    const host = repeatHost(
      currentItems,
      created,
      updated,
      removed,
      inserted,
    );
    host.$resolveValue = () => currentItems;
    const originalWarn = console.warn;
    let warningCount = 0;
    console.warn = () => {
      warningCount += 1;
    };

    try {
      syncRepeat(host, repeat);
      assert.deepEqual(repeat.instances, oldInstances);
      assert.deepEqual(repeat.keyState?.keys, []);
      assert.equal(warningCount, 1);

      currentItems = [{ id: 4 }, { id: 5 }];
      syncRepeat(host, repeat);
      assert.deepEqual(repeat.keyState?.keys, [4, 5]);

      currentItems = [{ id: 5 }, { id: 4 }];
      updated.length = 0;
      syncRepeat(host, repeat);
      assert.deepEqual(repeat.instances, [oldInstances[1], oldInstances[0]]);
      assert.equal(warningCount, 1);
    } finally {
      console.warn = originalWarn;
    }
  });

  test('supports primitive item keys without a property path', () => {
    const oldItems = ['a', 'b'];
    const newItems = ['b', 'a'];
    const oldInstances = oldItems.map(itemInstance);
    const repeat = keyedRepeatBinding(oldItems, oldInstances, '', oldItems);
    const host = repeatHost(newItems, [], [], [], []);

    syncRepeat(host, repeat);

    assert.deepEqual(repeat.instances, [oldInstances[1], oldInstances[0]]);
    assert.deepEqual(repeat.keyState?.keys, newItems);
  });

  test('seeds SSR identity from bootstrap collection order', () => {
    const items = [{ id: 1 }, { id: 2 }];
    const instances = items.map(itemInstance);
    const repeat = repeatBinding(items, instances);
    repeat.keyState = createRepeatKeyState('id');

    seedHydratedRepeatKeys(repeat, items);

    assert.deepEqual(repeat.keyState.keys, [1, 2]);
  });

  test('does not seed SSR identity when marker and data counts disagree', () => {
    const items = [{ id: 1 }, { id: 2 }];
    const instances = [itemInstance(items[0])];
    const repeat = repeatBinding(items.slice(0, 1), instances);
    repeat.keyState = createRepeatKeyState('id');

    seedHydratedRepeatKeys(repeat, items);

    assert.deepEqual(repeat.keyState.keys, []);
  });
});
