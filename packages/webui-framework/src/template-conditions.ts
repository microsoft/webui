// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type { CompiledCondition, CompiledConditionFn } from './template-types.js';

/** Coerce a resolved JSON condition operand using the server's truthiness rules. */
export function isConditionTruthy(value: unknown): boolean {
  if (!value || typeof value !== 'object') return !!value;
  return (Array.isArray(value) ? value.length : Object.keys(value).length) > 0;
}

type Resolver = Parameters<CompiledConditionFn>[0];

/** Run a compiler plan without allocating a stack or a closure per condition. */
export function evaluateCondition(condition: CompiledCondition, resolve: Resolver, scope: unknown): boolean {
  const [plan, paths] = condition;
  if (typeof plan === 'function') return plan(resolve, scope, isConditionTruthy);
  if (plan.length === 1) {
    const op = plan[0];
    if (op !== 0 && op !== 1) throw invalidPlan();
    return isConditionTruthy(resolve(paths[0], scope)) !== (op === 1);
  }

  // The compiler emits dependencies in reverse execution order. Each jump
  // points strictly backwards, or to one of the two terminal values.
  const single = plan.length === 3;
  let node = single ? 0 : plan.length / 5 - 1;
  for (;;) {
    const offset = node * 5;
    const op = plan[offset];
    if (typeof op !== 'number' || (op >>> 0) !== op || op > 14 || op === 8) break;
    const argument = plan[offset + 2];
    let result: boolean;
    if (op === 7) {
      result = argument === true;
    } else {
      const left = resolve(paths[plan[offset + 1] as number], scope);
      const right = op & 8 ? resolve(paths[argument as number], scope) : argument;
      // Preserve the compiler's existing browser coercion for comparisons.
      switch (op & 7) {
        case 0: result = isConditionTruthy(left); break;
        case 1: result = (left as number) > (right as number); break;
        case 2: result = (left as number) < (right as number); break;
        case 3: result = left == right; break;
        case 4: result = left != right; break;
        case 5: result = (left as number) >= (right as number); break;
        case 6: result = (left as number) <= (right as number); break;
        default: throw invalidPlan();
      }
    }
    if (single) return result;
    const next = plan[offset + (result ? 3 : 4)] as number;
    if (next === -2) return true;
    if (next === -1) return false;
    if (next < 0 || next >= node || (next | 0) !== next) break;
    node = next;
  }
  throw invalidPlan();
}

function invalidPlan(): Error {
  return new Error('[WebUI] Invalid compiled condition plan. Rebuild templates.');
}
