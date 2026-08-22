// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Hydration benchmark fixture.
 *
 * The `bench` fixture measures `$update()` cost once a component is live.
 * This one measures the other half: what it costs to adopt server-rendered
 * DOM in the first place.
 *
 * `test-hydration-wide` is deliberately flat and wide (50 bindings across 50
 * sibling nodes) because that is the shape that stresses SSR node lookup.
 * `test-hydration-deep` nests bindings inside static elements, and
 * `test-hydration-nested` chains `<if>` blocks so structural nesting depth -
 * the one dimension that is not linear - stays measurable.
 * `test-hydration-slots` interleaves text and conditionals at one static slot
 * so cross-kind boundary lookup stays linear. Each instance brackets its own hydration and accumulates into
 * `window.__hydrationBench`, so the spec can report a stable per-instance
 * mean over many instances instead of a single noisy sample.
 */

import { WebUIElement, observable } from '../../../src/index.js';

interface HydrationBenchTotals {
  totalMs: number;
  count: number;
}

function record(tag: string, ms: number): void {
  const w = window as unknown as Record<string, Record<string, HydrationBenchTotals>>;
  const store = (w.__hydrationBench ??= {});
  const entry = (store[tag] ??= { totalMs: 0, count: 0 });
  entry.totalMs += ms;
  entry.count += 1;
}

class HydrationTimed extends WebUIElement {
  @observable p0 = 'v0';
  @observable p1 = 'v1';
  @observable p2 = 'v2';
  @observable p3 = 'v3';
  @observable p4 = 'v4';
  @observable p5 = 'v5';
  @observable flag0 = true;
  @observable flag1 = false;
  @observable flag2 = true;

  override connectedCallback(): void {
    const start = performance.now();
    super.connectedCallback();
    record(this.tagName.toLowerCase(), performance.now() - start);
  }
}

export class TestHydrationWide extends HydrationTimed {}
export class TestHydrationDeep extends HydrationTimed {}
/** Structural nesting: a chain of `<if>` blocks, each hydrating the next. */
export class TestHydrationNested extends HydrationTimed {}
/** Shared static slot: interleaved text and conditional boundaries. */
export class TestHydrationSlots extends HydrationTimed {}

TestHydrationWide.define('test-hydration-wide');
TestHydrationDeep.define('test-hydration-deep');
TestHydrationNested.define('test-hydration-nested');
TestHydrationSlots.define('test-hydration-slots');
