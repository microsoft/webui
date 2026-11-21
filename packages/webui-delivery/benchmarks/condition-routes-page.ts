// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { observable, WebUIElement } from '@microsoft/webui-framework';
import type { CompiledConditionFn } from '../../webui-framework/src/template-types.js';

declare const __BENCH_DIRECT__: boolean;
declare const __BENCH_STATUS__: boolean;

type RowState = { count: number; status: string };

class ComparisonPage extends WebUIElement {
  @observable rows: RowState[] = [];
}

class ComparisonRow extends WebUIElement {
  @observable count = 0;
  @observable status = '';
}

ComparisonPage.define('bench-comparison-page');
ComparisonRow.define('bench-comparison-row');

const direct: CompiledConditionFn = __BENCH_STATUS__
  ? (resolve, scope) => resolve('status', scope) == 'active'
  : (resolve, scope) => (resolve('count', scope) as number) > 0;

export function prepare(partial: unknown): void {
  if (__BENCH_DIRECT__) {
    // Only this benchmark replaces the compiler plan with a function from the
    // already-required route module. The JSON carries paths, never JS source.
    const entry = (partial as { templates: Record<string, { c: [[
      [null | CompiledConditionFn, string[]],
    ]] }> }).templates['bench-comparison-row'].c[0][0];
    if (entry[0] !== null) throw new Error('Expected compiler plan placeholder in the direct control.');
    entry[0] = direct;
  }
}

export async function measureInteraction(start: number): Promise<{ elapsed: number; rows: number }> {
  const host = document.querySelector<ComparisonPage>('bench-comparison-page');
  if (!host) throw new Error('Route committed without its page element.');
  await host.$whenRendered();
  const children = host.querySelectorAll<ComparisonRow>('bench-comparison-row');
  const first = children[0];
  if (!first || host.querySelector('.matched')) throw new Error('Unexpected initial route DOM.');
  if (__BENCH_STATUS__) first.status = 'active';
  else first.count = 1;
  await host.$whenRendered();
  const elapsed = performance.now() - start;
  if (!first.querySelector('.matched')) throw new Error('Cold route did not react.');
  return { elapsed, rows: children.length };
}
