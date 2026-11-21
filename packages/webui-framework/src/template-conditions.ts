// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type { CompiledCondition, CompiledConditionFn, ConditionPlan } from './template-types.js';

/** Coerce a resolved JSON condition operand using the server's truthiness rules. */
export function isConditionTruthy(value: unknown): boolean {
  if (typeof value !== 'object' || value === null) return !!value;
  return (Array.isArray(value) ? value.length : Object.keys(value).length) > 0;
}

type Resolver = Parameters<CompiledConditionFn>[0];

/** Check the shape and operands of a plan before accepting its template. */
export function validateCondition(condition: unknown): void {
  if (!Array.isArray(condition) || condition.length !== 2) throw invalidPlan();
  const [plan, paths]: readonly unknown[] = condition;
  if (!Array.isArray(paths)) throw invalidPlan();
  for (const path of paths) {
    if (typeof path !== 'string') throw invalidPlan();
  }
  if (typeof plan === 'function') return;
  if (!Array.isArray(plan)) throw invalidPlan();
  const size = plan.length;
  if (size === 1) {
    if ((plan[0] !== 0 && plan[0] !== 1) || paths.length === 0) throw invalidPlan();
    return;
  }
  if (size !== 3 && (size === 0 || size % 5 !== 0)) throw invalidPlan();
  for (let offset = 0; offset < size; offset += 5) {
    const op: unknown = plan[offset];
    const right: unknown = plan[offset + 2];
    if (!indexIn(op, 0, 15) || op === 8) throw invalidPlan();
    if (op === 7 ? typeof right !== 'boolean' : !indexIn(plan[offset + 1], 0, paths.length)) {
      throw invalidPlan();
    }
    if (op & 8 ? !indexIn(right, 0, paths.length) :
        typeof right !== 'string' && typeof right !== 'boolean' && !Number.isFinite(right)) {
      throw invalidPlan();
    }
    // Check both targets, including the branch that current state would skip.
    const node = offset / 5;
    if (size > 3 && (!indexIn(plan[offset + 3], -2, node) || !indexIn(plan[offset + 4], -2, node))) {
      throw invalidPlan();
    }
  }
}

function indexIn(value: unknown, start: number, end: number): value is number {
  return typeof value === 'number' && Number.isInteger(value) && value >= start && value < end;
}

/** Run a compiler plan without allocating a stack or a closure per condition. */
export function evaluateCondition(condition: CompiledCondition, resolve: Resolver, scope: unknown): boolean {
  const plan = condition[0];
  if (typeof plan === 'function') return plan(resolve, scope, isConditionTruthy);
  const paths = condition[1];
  if (plan.length === 1) {
    const op = plan[0];
    if (op !== 0 && op !== 1) throw invalidPlan();
    return isConditionTruthy(resolve(paths[0], scope)) !== (op === 1);
  }
  // Keep single tests out of the branching interpreter so hot callers can inline them.
  if (plan.length === 3) return evaluateTest(plan, 0, paths, resolve, scope);
  return evaluateBranches(plan, paths, resolve, scope);
}

function evaluateBranches(plan: ConditionPlan, paths: string[], resolve: Resolver, scope: unknown): boolean {
  // The compiler emits dependencies in reverse execution order. Each jump
  // points strictly backwards, or to one of the two terminal values.
  let node = plan.length / 5 - 1;
  for (;;) {
    const offset = node * 5;
    const result = evaluateTest(plan, offset, paths, resolve, scope);
    const next = plan[offset + (result ? 3 : 4)] as number;
    if (next === -2 || next === -1) return next === -2;
    if (next < 0 || next >= node || (next | 0) !== next) break;
    node = next;
  }
  throw invalidPlan();
}

function evaluateTest(plan: ConditionPlan, offset: number, paths: string[], resolve: Resolver, scope: unknown): boolean {
  const op = plan[offset] as number;
  if ((op & 15) !== op || op > 14 || op === 8) throw invalidPlan();
  const argument = plan[offset + 2];
  if (op === 7) return argument === true;
  const left = resolve(paths[plan[offset + 1] as number], scope);
  const right = op & 8 ? resolve(paths[argument as number], scope) : argument;
  // Preserve the compiler's existing browser coercion for comparisons.
  switch (op & 7) {
    case 0: return isConditionTruthy(left);
    case 1: return (left as number) > (right as number);
    case 2: return (left as number) < (right as number);
    case 3: return left == right;
    case 4: return left != right;
    case 5: return (left as number) >= (right as number);
    case 6: return (left as number) <= (right as number);
    default: throw invalidPlan();
  }
}

function invalidPlan(): Error {
  return new Error('[WebUI] Invalid compiled condition plan. Rebuild templates.');
}
