// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';
import { createFragmentWork, MAX_FRAGMENT_DEPTH, MAX_FRAGMENT_VISITS } from './fragment-work.js';
import type { TemplateInstance, TextBinding } from './types.js';

function instance(order = 0): TemplateInstance {
  return {
    nodes: [], texts: [], attrs: [], conds: [], repeats: [], container: null,
    alive: true, order,
  };
}

describe('fragment operation work', () => {
  test('known-state policy reaches appended work without being retained by the controller', () => {
    const work = createFragmentWork();
    const keys = Object.keys(work);
    const flags: Array<boolean | undefined> = [];
    work.begin();
    work.enqueue(instance());
    work.drain({
      $processFragmentTask(_task, requireKnownState) {
        flags.push(requireKnownState);
        if (flags.length === 1) work.enqueue(instance());
      },
    }, true);
    work.end();
    work.begin();
    work.enqueue(instance());
    work.drain({
      $processFragmentTask(_task, requireKnownState) {
        flags.push(requireKnownState);
      },
    });
    work.end();
    assert.deepEqual(flags, [true, true, undefined]);
    assert.deepEqual(Object.keys(work), keys);
  });

  test('reuses one implementation while isolating stacks, generations and held budgets', () => {
    const first = createFragmentWork();
    const second = createFragmentWork();
    assert.equal(Object.getPrototypeOf(first), Object.getPrototypeOf(second));
    assert.equal(first.begin, second.begin);
    assert.equal(first.drain, second.drain);
    assert.notEqual(first.stack, second.stack);

    first.holdBudget();
    first.visit(1);
    first.begin();
    first.enqueue(instance());
    first.nextPass();
    assert.equal(second.active, false);
    assert.equal(second.generation, 0);
    assert.equal(second.visits, 0);
    assert.deepEqual(second.stack, []);

    second.begin();
    second.visit(1);
    second.end();
    second.begin();
    assert.equal(second.visits, 0, 'another owner does not inherit the held budget');
    second.end();
    assert.equal(first.active, true);
    assert.equal(first.generation, 2);
    assert.equal(first.visits, 1);
    assert.equal(first.stack.length, 1);
    first.end();
    first.releaseBudget();

    const later = createFragmentWork();
    assert.equal(Object.getPrototypeOf(later), Object.getPrototypeOf(first));
    assert.equal(later.generation, 0);
    assert.equal(later.visits, 0);
    assert.deepEqual(later.stack, []);
  });

  test('nested work does not reset the operation budget', () => {
    const work = createFragmentWork();
    assert.equal(work.begin(), true);
    work.visit(1);
    assert.equal(work.begin(), false);
    assert.equal(work.visits, 1);
    work.nextPass();
    assert.equal(work.visits, 1);
    work.end();
    work.begin();
    assert.equal(work.visits, 0);
  });

  test('permits exact limits and rejects the next invocation or active call', () => {
    const work = createFragmentWork();
    work.begin();
    for (let i = 0; i < MAX_FRAGMENT_VISITS; i++) work.visit(MAX_FRAGMENT_DEPTH);
    assert.throws(() => work.visit(1), /100000/);
    work.end();
    work.begin();
    assert.throws(() => work.visit(MAX_FRAGMENT_DEPTH + 1), /256/);
  });

  test('drains deep dynamically appended work without recursion', () => {
    const work = createFragmentWork();
    const items = Array.from({ length: 20_000 }, (_, i) => instance(i));
    work.begin();
    work.enqueue(items[0]);
    let count = 0;
    work.drain({
      $processFragmentTask(task) {
        assert.equal(task, items[count++]);
        if (count < items.length) work.enqueue(items[count]);
      },
    });
    assert.equal(count, items.length);
    assert.equal(work.stack.length, 0);
  });

  test('parents run first and duplicate/disposed work is skipped', () => {
    const work = createFragmentWork();
    const parent = instance(0);
    const child = instance(1);
    const text: TextBinding = { node: {} as CharacterData, owner: child };
    work.begin();
    work.enqueue(text);
    work.enqueue(text);
    work.enqueue(parent);
    work.sort();
    const seen: unknown[] = [];
    work.drain({
      $processFragmentTask(task) {
        seen.push(task);
        if (task === parent) child.alive = false;
      },
    });
    assert.deepEqual(seen, [parent]);
  });

  test('generation permits a later pass without retaining stale queue entries', () => {
    const work = createFragmentWork();
    const item = instance();
    let calls = 0;
    const host = { $processFragmentTask: () => { calls++; } };
    work.begin();
    work.enqueue(item);
    work.enqueue(item);
    work.drain(host);
    assert.equal(calls, 1);
    work.nextPass();
    work.enqueue(item);
    work.drain(host);
    assert.equal(calls, 2);
    work.enqueue(instance());
    work.end();
    assert.equal(work.stack.length, 0);
  });

  test('mount hydration and replay share a held invocation budget', () => {
    const work = createFragmentWork();
    work.holdBudget();
    work.visit(1);
    work.begin();
    assert.equal(work.visits, 1);
    work.visit(2);
    work.end();
    work.begin();
    assert.equal(work.visits, 2);
    work.end();
    work.releaseBudget();
    work.begin();
    assert.equal(work.visits, 0);
  });
});
