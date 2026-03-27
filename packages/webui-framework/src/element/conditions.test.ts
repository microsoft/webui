// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';

import type { CompiledConditionExpr } from '../template.js';
import {
  conditionUsesPath,
  deriveConditionSeed,
  evaluateCondition,
} from './conditions.js';

describe('condition helpers', () => {
  test('evaluateCondition handles compound conditions and literals', () => {
    const condition: CompiledConditionExpr = [3, [0, 'isOpen'], 1, [1, 'count', 5, '3']];
    const values: Record<string, unknown> = {
      isOpen: true,
      count: 3,
    };

    assert.equal(evaluateCondition(condition, (path) => values[path]), true);
  });

  test('evaluateCondition short-circuits logical branches', () => {
    const condition: CompiledConditionExpr = [3, [0, 'enabled'], 1, [0, 'expensive']];
    const resolved: string[] = [];
    const values: Record<string, unknown> = {
      enabled: false,
      expensive: true,
    };

    const result = evaluateCondition(condition, (path) => {
      resolved.push(path);
      return values[path];
    });

    assert.equal(result, false);
    assert.deepEqual(resolved, ['enabled']);
  });

  test('conditionUsesPath finds nested identifiers and predicate operands', () => {
    const condition: CompiledConditionExpr = [3, [1, 'page', 3, 'activePage'], 2, [2, [0, 'hidden']]];

    assert.equal(conditionUsesPath(condition, 'page'), true);
    assert.equal(conditionUsesPath(condition, 'activePage'), true);
    assert.equal(conditionUsesPath(condition, 'hidden'), true);
    assert.equal(conditionUsesPath(condition, 'missing'), false);
  });

  test('deriveConditionSeed infers boolean, literal, and empty collection seeds', () => {
    assert.deepEqual(deriveConditionSeed([0, 'open'], false), {
      kind: 'path',
      path: 'open',
      value: false,
    });

    assert.deepEqual(deriveConditionSeed([2, [0, 'disabled']], true), {
      kind: 'path',
      path: 'disabled',
      value: false,
    });

    assert.deepEqual(deriveConditionSeed([1, 'state', 3, "'done'"], true), {
      kind: 'path',
      path: 'state',
      value: 'done',
    });

    assert.deepEqual(deriveConditionSeed([1, 'items.length', 3, '0'], true), {
      kind: 'empty-collection',
      path: 'items',
    });
  });
});
