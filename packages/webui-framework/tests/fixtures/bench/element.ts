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
import {
  bindAttr,
  bindBoolAttr,
  bindEvent,
  bindText,
  dynamic,
  identifier,
  nodePath,
  registerCompiledTemplate,
  repeat,
  slot,
} from '@microsoft/webui-test-support';

// Build a template with many bindings.
// Structure: 10 <p> elements each showing prop0..prop9,
//            10 <span> elements with attr bindings,
//            5 <div> with boolean disabled attrs,
//            plus a button to trigger measurement.

const textBindings = [];
const htmlParts: string[] = [];

// 50 text bindings: 5 per property, 10 properties
for (let prop = 0; prop < 10; prop++) {
  for (let dup = 0; dup < 5; dup++) {
    const idx = prop * 5 + dup;
    htmlParts.push(`<span class="t${idx}"></span>`);
    textBindings.push(
      bindText(slot({ parent: nodePath(idx), before: 0 }), dynamic(`prop${prop}`)),
    );
  }
}

// 10 attribute bindings on elements 50-59
const attrBindings = [];
const attrGroups = [];
for (let i = 0; i < 10; i++) {
  const elIdx = 50 + i;
  htmlParts.push(`<span class="a${i}"></span>`);
  attrBindings.push(bindAttr('data-val', `prop${i}`));
  attrGroups.push({ target: nodePath(elIdx), startIndex: i, bindingCount: 1 });
}

// 5 boolean attribute bindings on elements 60-64
for (let i = 0; i < 5; i++) {
  const elIdx = 60 + i;
  htmlParts.push(`<span class="b${i}"></span>`);
  attrBindings.push(bindBoolAttr('disabled', identifier(`prop${i}`)));
  attrGroups.push({ target: nodePath(elIdx), startIndex: 10 + i, bindingCount: 1 });
}

// Button and result display
htmlParts.push('<button class="run"></button>');
htmlParts.push('<pre class="result"></pre>');

registerCompiledTemplate('test-bench', {
  h: htmlParts.join(''),
  text: textBindings,
  attrs: attrBindings,
  attrGroups: attrGroups.map(g => ({
    target: g.target,
    startIndex: g.startIndex,
    bindingCount: g.bindingCount,
  })),
  events: [bindEvent('click', 'runBenchmark')],
  eventTargets: [nodePath(65)],
});

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

registerCompiledTemplate('test-bench-item', {
  h: '<li class="bench-item"><span class="label"></span> — <span class="value"></span></li>',
  text: [
    bindText(slot({ parent: nodePath(0, 0), before: 0 }), dynamic('item.label')),
    bindText(slot({ parent: nodePath(0, 2), before: 0 }), dynamic('item.value')),
  ],
});

registerCompiledTemplate('test-bench-repeat', {
  h: '<button class="run-repeat"></button><ul class="list"></ul>',
  repeats: [repeat('items', 'item', { blockIndex: 0 })],
  repeatSlots: [slot({ parent: nodePath(1), before: 0 })],
  blocks: [{
    h: '<li class="bench-item"><span class="label"></span> — <span class="value"></span></li>',
    text: [
      bindText(slot({ parent: nodePath(0, 0), before: 0 }), dynamic('item.label')),
      bindText(slot({ parent: nodePath(0, 2), before: 0 }), dynamic('item.value')),
    ],
  }],
  events: [bindEvent('click', 'runRepeatBench')],
  eventTargets: [nodePath(0)],
});

interface BenchItem {
  label: string;
  value: string;
}

export class TestBenchRepeat extends WebUIElement {
  @observable items: BenchItem[] = [];
  @observable benchResult = '';

  runRepeatBench(): void {
    // Generate 200 items
    const data: BenchItem[] = [];
    for (let i = 0; i < 200; i++) {
      data.push({ label: `Item ${i}`, value: `val-${i}` });
    }

    // Benchmark: create 200 items (measures template cloning vs parsing)
    const createStart = performance.now();
    this.items = data;
    const createTime = performance.now() - createStart;

    // Benchmark: update all items (swap values)
    const swapped = data.map(d => ({ label: d.value, value: d.label }));
    const updateStart = performance.now();
    this.items = swapped;
    const updateTime = performance.now() - updateStart;

    // Benchmark: clear all
    const clearStart = performance.now();
    this.items = [];
    const clearTime = performance.now() - clearStart;

    // Second create to measure cached clone path
    const create2Start = performance.now();
    this.items = data;
    const create2Time = performance.now() - create2Start;

    const result = {
      itemCount: 200,
      createMs: Math.round(createTime * 100) / 100,
      create2Ms: Math.round(create2Time * 100) / 100,
      updateMs: Math.round(updateTime * 100) / 100,
      clearMs: Math.round(clearTime * 100) / 100,
    };

    this.benchResult = JSON.stringify(result, null, 2);
    (window as unknown as Record<string, unknown>).__repeatBenchResult = result;
  }
}

TestBenchRepeat.define('test-bench-repeat');

// ── Event closure benchmark ────────────────────────────────────────
// Each repeat item has 5 event bindings → 200 items × 5 = 1000 closures

registerCompiledTemplate('test-bench-events', {
  h: '<button class="run-events"></button><div class="event-list"></div>',
  repeats: [repeat('items', 'item', { blockIndex: 0 })],
  repeatSlots: [slot({ parent: nodePath(1), before: 0 })],
  blocks: [{
    h: '<div class="event-item"><button class="a"></button><button class="b"></button><button class="c"></button><button class="d"></button><button class="e"></button></div>',
    events: [
      bindEvent('click', 'onA'),
      bindEvent('click', 'onB'),
      bindEvent('click', 'onC'),
      bindEvent('click', 'onD'),
      bindEvent('click', 'onE'),
    ],
    eventTargets: [nodePath(0, 0), nodePath(0, 1), nodePath(0, 2), nodePath(0, 3), nodePath(0, 4)],
  }],
  events: [bindEvent('click', 'runEventBench')],
  eventTargets: [nodePath(0)],
});

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
