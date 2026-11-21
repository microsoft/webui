// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { evaluateCondition } from './template-conditions.js';

test('condition plans use scoped values, collection truthiness, and short-circuit branches', () => {
  const scope = { ready: false, items: [], count: 2, limit: 1 };
  const reads: string[] = [];
  const resolve = (path: string, value: unknown) => {
    assert.equal(value, scope);
    reads.push(path);
    return scope[path as keyof typeof scope];
  };
  assert.equal(evaluateCondition([[0], ['items']], resolve, scope), false);
  assert.equal(evaluateCondition([[1], ['items']], resolve, scope), true);

  // ready && count > limit: the left term jumps directly to false when absent.
  const plan = [9, 0, 1, -2, -1, 0, 2, false, 0, -1];
  const paths = ['count', 'limit', 'ready'];
  assert.equal(evaluateCondition([plan, paths], resolve, scope), false);
  assert.deepEqual(reads, ['items', 'items', 'ready']);
  scope.ready = true;
  assert.equal(evaluateCondition([plan, paths], resolve, scope), true);
  assert.deepEqual(reads.slice(3), ['ready', 'count', 'limit']);
});

test('a malformed condition cannot loop or fall through as a successful evaluation', () => {
  for (const plan of [[0, 0, false, 0, -1], [2], [16, 0, false], [8, 0, 0], [1.5, 0, 0]]) {
    assert.throws(() => evaluateCondition([plan, ['ready']], () => true, undefined),
      /Invalid compiled condition plan/);
  }
});

test('comparison plans select literal and scoped path operands', () => {
  const cases = [
    [1, [false, true, false, true, false, true]],
    [2, [false, false, true, false, true, true]],
    [3, [true, false, false, true, true, false]],
  ] as const;
  for (const [i, op] of [1, 2, 3, 4, 5, 6].entries()) {
    for (const [value, expected] of cases) {
      const resolve = (path: string) => path === 'value' ? value : 2;
      assert.equal(evaluateCondition([[op, 0, 2], ['value']], resolve, undefined), expected[i]);
      assert.equal(evaluateCondition([[op | 8, 0, 1], ['value', 'limit']], resolve, undefined), expected[i]);
    }
  }
});

test('compiled constants and negated comparisons terminate without following a fallthrough', () => {
  const resolve = () => 100;
  assert.equal(evaluateCondition([[7, 0, true], []], resolve, undefined), true);
  assert.equal(evaluateCondition([[7, 0, false, -1, -2], []], resolve, undefined), true);
  assert.equal(evaluateCondition([[3, 0, 100, -1, -2], ['count']], resolve, undefined), false);
});
