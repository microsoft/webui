// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Condition expression evaluation for the WebUI runtime.
 *
 * The Rust compiler emits conditional metadata as compact AST tuples:
 *
 *   [0, path]                      — identifier (truthy check)
 *   [1, left, operator, right]     — comparison predicate
 *   [2, inner]                     — logical NOT
 *   [3, left, operator, right]     — compound AND/OR
 *
 * This module evaluates those tuples at runtime using an iterative stack
 * (no recursion) to avoid call-stack depth in hot update paths.
 *
 * It also provides `deriveConditionSeed` which infers an observable's initial
 * value from whether its conditional block is shown or hidden in SSR HTML
 * (e.g. `@if(visible)` shown → `this.visible = true`).
 */

import type {
  CompiledComparisonOperator,
  CompiledConditionExpr,
  CompiledLogicalOperator,
} from '../template.js';
import type { ScopeFrame } from './types.js';

export type ConditionResolver = (path: string, scope?: ScopeFrame) => unknown;

export type ConditionSeed =
  | {
    kind: 'path';
    path: string;
    value: unknown;
  }
  | {
    kind: 'empty-collection';
    path: string;
  };

const LOGICAL_AND: CompiledLogicalOperator = 1;
const LOGICAL_OR: CompiledLogicalOperator = 2;

const GREATER_THAN: CompiledComparisonOperator = 1;
const LESS_THAN: CompiledComparisonOperator = 2;
const EQUAL: CompiledComparisonOperator = 3;
const NOT_EQUAL: CompiledComparisonOperator = 4;
const GREATER_THAN_OR_EQUAL: CompiledComparisonOperator = 5;
const LESS_THAN_OR_EQUAL: CompiledComparisonOperator = 6;

interface EvaluationFrame {
  condition: CompiledConditionExpr;
  stage: 0 | 1 | 2;
  left?: boolean;
}

export function evaluateCondition(
  condition: CompiledConditionExpr,
  resolveValue: ConditionResolver,
  scope?: ScopeFrame,
): boolean {
  const frames: EvaluationFrame[] = [{ condition, stage: 0 }];
  const results: boolean[] = [];

  while (frames.length > 0) {
    const frame = frames.pop();
    if (!frame) {
      break;
    }

    const current = frame.condition;
    switch (current[0]) {
      case 0:
        results.push(isTruthy(resolveValue(current[1], scope)));
        break;
      case 1:
        results.push(evaluatePredicate(current, resolveValue, scope));
        break;
      case 2:
        if (frame.stage === 0) {
          frames.push({ condition: current, stage: 1 });
          frames.push({ condition: current[1], stage: 0 });
          break;
        }

        results.push(!popBoolean(results));
        break;
      case 3:
        if (frame.stage === 0) {
          frames.push({ condition: current, stage: 1 });
          frames.push({ condition: current[1], stage: 0 });
          break;
        }

        if (frame.stage === 1) {
          const left = popBoolean(results);
          if (current[2] === LOGICAL_AND && !left) {
            results.push(false);
            break;
          }

          if (current[2] === LOGICAL_OR && left) {
            results.push(true);
            break;
          }

          frames.push({ condition: current, stage: 2, left });
          frames.push({ condition: current[3], stage: 0 });
          break;
        }

        results.push(
          current[2] === LOGICAL_AND
            ? !!frame.left && popBoolean(results)
            : !!frame.left || popBoolean(results),
        );
        break;
      default:
        results.push(false);
        break;
    }
  }

  return results.length > 0 ? results[results.length - 1] : false;
}

export function conditionUsesPath(condition: CompiledConditionExpr, path: string): boolean {
  const stack: CompiledConditionExpr[] = [condition];

  while (stack.length > 0) {
    const current = stack.pop();
    if (!current) {
      continue;
    }

    switch (current[0]) {
      case 0:
        if (current[1] === path) {
          return true;
        }
        break;
      case 1:
        if (current[1] === path) {
          return true;
        }

        if (!isLiteralValue(current[3]) && current[3] === path) {
          return true;
        }
        break;
      case 2:
        stack.push(current[1]);
        break;
      case 3:
        stack.push(current[3]);
        stack.push(current[1]);
        break;
      default:
        break;
    }
  }

  return false;
}

export function deriveConditionSeed(
  condition: CompiledConditionExpr,
  shown: boolean,
): ConditionSeed | null {
  let current = condition;
  let visible = shown;

  while (current[0] === 2) {
    current = current[1];
    visible = !visible;
  }

  if (current[0] === 0) {
    return identifierSeed(current[1], visible);
  }

  if (current[0] !== 1) {
    return null;
  }

  const [_, left, operator, right] = current;
  const literal = parseLiteral(right);
  if (literal === undefined) {
    return null;
  }

  if (isEmptyLengthSeed(left, operator, literal, visible)) {
    return { kind: 'empty-collection', path: left.slice(0, -'.length'.length) };
  }

  if (
    (operator === EQUAL && visible)
    || (operator === NOT_EQUAL && !visible)
  ) {
    return { kind: 'path', path: left, value: literal };
  }

  return null;
}

function identifierSeed(path: string, shown: boolean): ConditionSeed | null {
  if (path.endsWith('.length')) {
    if (!shown) {
      return { kind: 'empty-collection', path: path.slice(0, -'.length'.length) };
    }

    return null;
  }

  return { kind: 'path', path, value: shown };
}

function isEmptyLengthSeed(
  path: string,
  operator: CompiledComparisonOperator,
  literal: unknown,
  shown: boolean,
): boolean {
  if (!path.endsWith('.length') || literal !== 0) {
    return false;
  }

  return (operator === EQUAL && shown) || (operator === NOT_EQUAL && !shown);
}

function evaluatePredicate(
  condition: Extract<CompiledConditionExpr, [1, string, CompiledComparisonOperator, string]>,
  resolveValue: ConditionResolver,
  scope?: ScopeFrame,
): boolean {
  const left = resolveValue(condition[1], scope);
  const right = resolvePredicateValue(condition[3], resolveValue, scope);
  return compareValues(left, condition[2], right);
}

function resolvePredicateValue(
  value: string,
  resolveValue: ConditionResolver,
  scope?: ScopeFrame,
): unknown {
  const literal = parseLiteral(value);
  if (literal !== undefined) {
    return literal;
  }

  return resolveValue(value, scope);
}

function compareValues(
  left: unknown,
  operator: CompiledComparisonOperator,
  right: unknown,
): boolean {
  switch (operator) {
    case EQUAL:
      return Object.is(left, right);
    case NOT_EQUAL:
      return !Object.is(left, right);
    case GREATER_THAN:
      return compareOrdered(left, right, (a, b) => a > b);
    case LESS_THAN:
      return compareOrdered(left, right, (a, b) => a < b);
    case GREATER_THAN_OR_EQUAL:
      return compareOrdered(left, right, (a, b) => a >= b);
    case LESS_THAN_OR_EQUAL:
      return compareOrdered(left, right, (a, b) => a <= b);
    default:
      return false;
  }
}

function compareOrdered(
  left: unknown,
  right: unknown,
  compare: (left: number, right: number) => boolean,
): boolean {
  const leftNumber = toNumber(left);
  const rightNumber = toNumber(right);
  if (leftNumber === undefined || rightNumber === undefined) {
    return false;
  }

  return compare(leftNumber, rightNumber);
}

function toNumber(value: unknown): number | undefined {
  if (typeof value === 'number') {
    return Number.isNaN(value) ? undefined : value;
  }

  if (typeof value === 'string') {
    const parsed = Number(value);
    return Number.isNaN(parsed) ? undefined : parsed;
  }

  if (typeof value === 'boolean') {
    return value ? 1 : 0;
  }

  return undefined;
}

function isTruthy(value: unknown): boolean {
  if (typeof value === 'boolean') {
    return value;
  }

  if (value == null) {
    return false;
  }

  if (typeof value === 'number') {
    return value !== 0;
  }

  if (typeof value === 'string') {
    return value.length > 0;
  }

  if (Array.isArray(value)) {
    return value.length > 0;
  }

  if (typeof value === 'object') {
    return Object.keys(value as Record<string, unknown>).length > 0;
  }

  return false;
}

function isLiteralValue(value: string): boolean {
  return (
    value.startsWith('"')
    || value.startsWith('\'')
    || /^-?\d+(?:\.\d+)?$/.test(value)
    || value === 'true'
    || value === 'false'
  );
}

function parseLiteral(value: string): unknown {
  if (
    (value.startsWith('"') && value.endsWith('"'))
    || (value.startsWith('\'') && value.endsWith('\''))
  ) {
    return value.slice(1, -1);
  }

  if (value === 'true') {
    return true;
  }

  if (value === 'false') {
    return false;
  }

  if (/^-?\d+(?:\.\d+)?$/.test(value)) {
    const parsed = Number(value);
    if (!Number.isNaN(parsed)) {
      return parsed;
    }
  }

  return undefined;
}

function popBoolean(results: boolean[]): boolean {
  const value = results.pop();
  return value ?? false;
}
