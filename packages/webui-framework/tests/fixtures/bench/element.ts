// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Benchmark fixture — a component with many bindings to measure $update() cost.
 *
 * 50 text bindings across 10 observable properties (5 bindings each),
 * 10 attribute bindings, 5 boolean-conditional attributes, and 2 repeats.
 * This exercises every binding kind in the update loop.
 *
 * The benchmark mutates a single property and measures how long the
 * framework takes to run $update(), which currently walks ALL bindings
 * regardless of which property changed.
 */

import { WebUIElement, observable } from '../../../src/index.js';

export class TestBench extends WebUIElement {
  @observable prop0 = 'v0';
  @observable prop1 = 'v1';
  @observable prop2 = 'v2';
  @observable prop3 = 'v3';
  @observable prop4 = 'v4';
  @observable prop5 = 'v5';
  @observable prop6 = 'v6';
  @observable prop7 = 'v7';
  @observable prop8 = 'v8';
  @observable prop9 = 'v9';

  @observable benchResult = '';

  runBenchmark(): void {
    const iterations = 10_000;

    // Warm up
    for (let i = 0; i < 100; i++) {
      this.prop0 = `warm-${i}`;
    }

    // Benchmark: mutate ONE property, measure total $update() time
    const singleStart = performance.now();
    for (let i = 0; i < iterations; i++) {
      this.prop0 = `single-${i}`;
    }
    const singleTime = performance.now() - singleStart;

    // Benchmark: mutate ALL properties
    const allStart = performance.now();
    for (let i = 0; i < iterations; i++) {
      this.prop0 = `all-${i}`;
      this.prop1 = `all-${i}`;
      this.prop2 = `all-${i}`;
      this.prop3 = `all-${i}`;
      this.prop4 = `all-${i}`;
      this.prop5 = `all-${i}`;
      this.prop6 = `all-${i}`;
      this.prop7 = `all-${i}`;
      this.prop8 = `all-${i}`;
      this.prop9 = `all-${i}`;
    }
    const allTime = performance.now() - allStart;

    const result = {
      iterations,
      bindings: { text: 50, attr: 10, boolAttr: 5, total: 65 },
      singlePropMs: Math.round(singleTime * 100) / 100,
      singlePropPerUpdate: Math.round((singleTime / iterations) * 10000) / 10000,
      allPropsMs: Math.round(allTime * 100) / 100,
      allPropsPerUpdate: Math.round((allTime / (iterations * 10)) * 10000) / 10000,
    };

    this.benchResult = JSON.stringify(result, null, 2);

    // Also expose on window for Playwright to read
    (window as unknown as Record<string, unknown>).__benchResult = result;
  }
}

TestBench.define('test-bench');

// ── Repeat instantiation benchmark ─────────────────────────────────

interface BenchItem {
  id: number;
  label: string;
  value: string;
}

export class TestBenchRepeat extends WebUIElement {
  @observable items: BenchItem[] = [];
  @observable benchResult = '';

  runRepeatBench(): void {
    const updateIterations = 200;
    const data: BenchItem[] = [];
    const swapped: BenchItem[] = [];
    for (let i = 0; i < 200; i++) {
      data.push({ id: i, label: `Item ${i}`, value: `val-${i}` });
      swapped.push({ id: i, label: `val-${i}`, value: `Item ${i}` });
    }

    const createStart = performance.now();
    this.items = data;
    this.$flushUpdates();
    const createTime = performance.now() - createStart;

    const updateStart = performance.now();
    for (let i = 0; i < updateIterations; i++) {
      this.items = i % 2 === 0 ? swapped : data;
      this.$flushUpdates();
    }
    const updateTime = performance.now() - updateStart;

    const clearStart = performance.now();
    this.items = [];
    this.$flushUpdates();
    const clearTime = performance.now() - clearStart;

    const create2Start = performance.now();
    this.items = data;
    this.$flushUpdates();
    const create2Time = performance.now() - create2Start;

    const result = {
      itemCount: 200,
      updateIterations,
      createMs: Math.round(createTime * 100) / 100,
      create2Ms: Math.round(create2Time * 100) / 100,
      updateMs: Math.round(updateTime * 100) / 100,
      updatePerIterationMs:
        Math.round((updateTime / updateIterations) * 1000) / 1000,
      clearMs: Math.round(clearTime * 100) / 100,
    };

    this.benchResult = JSON.stringify(result, null, 2);
    (window as unknown as Record<string, unknown>).__repeatBenchResult = result;
  }
}

TestBenchRepeat.define('test-bench-repeat');

// ── Event closure benchmark ────────────────────────────────────────
// Each repeat item has 5 event bindings → 200 items × 5 = 1000 closures

export class TestBenchEvents extends WebUIElement {
  @observable items: Array<{ id: number }> = [];

  onA(): void {}
  onB(): void {}
  onC(): void {}
  onD(): void {}
  onE(): void {}

  runEventBench(): void {
    const data: Array<{ id: number }> = [];
    for (let i = 0; i < 200; i++) {
      data.push({ id: i });
    }

    if (window.gc) window.gc();
    const perf = performance as unknown as Record<string, unknown>;
    const memBefore = (perf.memory as Record<string, number>)?.usedJSHeapSize ?? 0;

    const start = performance.now();
    this.items = data; // creates 200 items × 5 events = 1000 event listeners
    const elapsed = performance.now() - start;

    const memAfter = (perf.memory as Record<string, number>)?.usedJSHeapSize ?? 0;

    const result = {
      itemCount: 200,
      eventsPerItem: 5,
      totalListeners: 1000,
      createMs: Math.round(elapsed * 100) / 100,
      memDeltaKB: Math.round((memAfter - memBefore) / 1024),
    };

    (window as unknown as Record<string, unknown>).__eventBenchResult = result;
  }
}

TestBenchEvents.define('test-bench-events');
