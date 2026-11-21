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
  assert.throws(() => evaluateCondition([[0, 0, false, 0, -1], ['ready']], () => true, undefined),
    /Invalid compiled condition plan/);
});
