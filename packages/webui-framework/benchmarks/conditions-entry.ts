// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { observable, WebUIElement } from '../src/index.js';
import { startRuntime } from '../src/runtime.js';
import { evaluateCondition, isConditionTruthy } from '../src/template-conditions.js';
import { registerTemplateData } from '../src/template-registry.js';
import { TEMPLATE_META_VERSION, type CompiledConditionFn, type PlannedCondition } from '../src/template-types.js';

const cases: Record<string, readonly [PlannedCondition, CompiledConditionFn, boolean]> = {
  truthy: [[[0], ['count']], (resolve, scope, truthy) => truthy(resolve('count', scope)), true],
  negated: [[[1], ['count']], (resolve, scope, truthy) => !truthy(resolve('count', scope)), false],
  comparison: [[[1, 0, 0], ['count']], (resolve, scope) => (resolve('count', scope) as number) > 0, true],
  compound: [
    [[0, 0, false, -1, -2, 1, 1, 0, 0, -1], ['busy', 'count']],
    (resolve, scope, truthy) => (resolve('count', scope) as number) > 0 && !truthy(resolve('busy', scope)),
    true,
  ],
};

const name = new URLSearchParams(location.search).get('case');
if (!name || !Object.hasOwn(cases, name)) throw new Error('Missing benchmark case');
const [plan, direct, positive] = cases[name];
const scope = [{ count: 0, busy: false }, { count: 1, busy: false }];
type State = typeof scope[number];
const resolve = (path: string, state?: unknown): unknown => (state as State)[path as keyof State];
const compiled = [direct] as const;

for (let i = 0; i < scope.length; i++) {
  const expected = Boolean(i) === positive;
  if (direct(resolve, scope[i], isConditionTruthy) !== expected ||
      evaluateCondition(plan, resolve, scope[i]) !== expected) {
    throw new Error(`Wrong evaluator result for ${name}`);
  }
}

class ConditionBench extends WebUIElement {
  @observable count = 0;
  @observable busy = false;
}

startRuntime();
registerTemplateData({
  'condition-bench': {
    v: TEMPLATE_META_VERSION, sd: 1, h: '', c: [[plan, 0, [0, 0]]],
    b: [{ h: '<span id="active"></span>' }], tr: ['count', 'busy'],
  },
});
ConditionBench.define('condition-bench');
const host = new ConditionBench();
document.body.append(host);
await host.$whenRendered();

function checkDom(expected: boolean): void {
  if (Boolean(host.shadowRoot?.querySelector('#active')) !== expected) {
    throw new Error(`Wrong mounted condition for ${name}`);
  }
}
checkDom(!positive);

function runDirect(iterations: number): number {
  let sum = 0;
  for (let i = 0; i < iterations; i++) {
    sum += Number(compiled[0](resolve, scope[i & 1], isConditionTruthy));
  }
  return sum;
}

function runPlan(iterations: number): number {
  let sum = 0;
  for (let i = 0; i < iterations; i++) {
    sum += Number(evaluateCondition(plan, resolve, scope[i & 1]));
  }
  return sum;
}

declare global {
  interface Window {
    conditionBench: (kind: 'direct' | 'plan' | 'steady' | 'toggle', repeats: number) => Promise<number>;
  }
}
window.conditionBench = async (kind, repeats) => {
  if (kind === 'toggle' || kind === 'steady') {
    const offset = kind === 'steady' ? 1 : 0;
    host.count = 1;
    await host.$whenRendered();
    checkDom(positive);
    host.count = offset;
    await host.$whenRendered();
    checkDom(offset === 1 ? positive : !positive);
    const start = performance.now();
    for (let i = 0; i < repeats; i++) {
      host.count = 1 - (i & 1) + offset;
      await host.$whenRendered();
    }
    const elapsed = performance.now() - start;
    checkDom(offset === 1 ? positive : !positive);
    return elapsed * 1_000 / repeats;
  }
  const start = performance.now();
  const sum = kind === 'direct' ? runDirect(repeats) : runPlan(repeats);
  const elapsed = performance.now() - start;
  if (sum !== repeats / 2) throw new Error(`Wrong checksum for ${name}`);
  return elapsed * 1_000_000 / repeats;
};
