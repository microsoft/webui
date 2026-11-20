// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { readConditionTable } from './condition-table.js';

test('decodes the compiler table with matching scope, coercion, and short-circuit semantics', () => {
  const [present, compare, compound, negated, disjunction, escaped] = readConditionTable(
    '[function(v,s,t){return t(v("items",s))},' +
    'function(v,s){return v("count",s)>=-.5},' +
    'function(v,s,t){return (t(v("ready",s))&&v("count",s)==v("minimum",s))},' +
    'function(v,s,t){return !(t(v("ready",s)))},' +
    'function(v,s){return (true||v("count",s)<1)},' +
    String.raw`function(v,s){return v("label",s)=="say \"hi\"\n\u003C"}]`,
    'test-card',
  );
  const values: Record<string, unknown> = {
    items: [], count: '2', minimum: 2, ready: false, label: 'say "hi"\n<',
  };
  const reads: string[] = [];
  const get = (path: string, scope?: unknown) => {
    assert.equal(scope, values);
    reads.push(path);
    return values[path];
  };
  const truthy = (value: unknown) => Array.isArray(value) ? value.length > 0 : Boolean(value);
  assert.equal(present(get, values, truthy), false);
  assert.equal(compare(get, values, truthy), true);
  assert.equal(compound(get, values, truthy), false);
  assert.equal(disjunction(get, values, truthy), true);
  assert.deepEqual(reads, ['items', 'count', 'ready']);
  values.ready = true;
  assert.equal(compound(get, values, truthy), true);
  assert.equal(negated(get, values, truthy), false);
  assert.equal(escaped(get, values, truthy), true);
});

test('preserves odd and even runs of consecutive compiler negations', () => {
  // The evaluator's logical-operator limit counts && and ||, not unary !.
  const odd = `${'!('.repeat(33)}t(v("ready",s))${')'.repeat(33)}`;
  const even = `!(${odd})`;
  const [negated, affirmed] = readConditionTable(
    `[function(v,s,t){return ${odd}},function(v,s,t){return ${even}}]`,
    'test-card',
  );
  assert.equal(negated(() => true, undefined, Boolean), false);
  assert.equal(negated(() => false, undefined, Boolean), true);
  assert.equal(affirmed(() => true, undefined, Boolean), true);
  assert.equal(affirmed(() => false, undefined, Boolean), false);
});

test('rejects tables outside the compiler grammar and bounds compound nesting', () => {
  for (const table of [
    null,
    '[true]',
    '[function(v,s){return true}] []',
    '[function(v,s){return 1+2}]',
    '[function(v,s){return v("label",s)=="\\q"}]',
    `[function(v,s){return ${'(true&&'.repeat(33)}true${')'.repeat(33)}}]`,
  ]) {
    assert.throws(() => readConditionTable(table, 'test-card'), /Invalid compiler condition table/);
  }
});
