// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { existsSync, readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { expect, test } from '@playwright/test';
import type { CDPSession } from '@playwright/test';

import { adoptScenario } from './performance-adoption.js';
import type { AdoptSample } from './performance-adoption.js';
import { runScenario } from './performance-client.js';
import type { Sample, Scenario } from './performance-client.js';
import { renderSsr } from './performance-render.js';
import type { SsrRender } from './performance-render.js';

// Run this fixture alone with WEBUI_FRAGMENT_PERF=1 and --workers=1.
// Timing is reported, not gated: deterministic work/identity checks are the gate.
//
// Two distinct paths are measured and reported separately:
//   1. client creation - `document.createElement` builds the tree from scratch.
//   2. SSR adoption    - the pipeline renders real SSR HTML, the browser parses it
//                        with no framework code present, and only then is the
//                        current bundled entry injected so hydration is timed on
//                        its own instead of being folded into creation.
test.describe.configure({ mode: 'serial' });
test.use({
  launchOptions: {
    args: [
      '--enable-blink-features=DeclarativeCSSModules',
      '--enable-precise-memory-info',
    ],
  },
});

const FIXTURE = 'recursive-fragments';
const TAG = 'test-recursive-tree';
const BUNDLE_PATH = `/dist/${FIXTURE}/element.js`;
const SSR_ROUTE = `/${FIXTURE}/perf-ssr.html`;
const here = dirname(fileURLToPath(import.meta.url));
const fixturesRoot = resolve(here, '..');
const projectionManifest = resolve(fixturesRoot, 'dist', 'webui-projection.json');
const defaultState = resolve(here, 'state.json');

const CLIENT_SCENARIOS: Scenario[] = [
  { shape: 'deep', size: 16 },
  { shape: 'deep', size: 64 },
  { shape: 'deep', size: 128 },
  { shape: 'deep', size: 240 },
  { shape: 'wide', size: 100 },
  { shape: 'wide', size: 1000 },
];

// SSR nests one JSON object plus one array per level, so deep trees approach the
// native JSON depth limit long before the client does. Every candidate is probed
// once during setup; unsupported ones are reported explicitly, never skipped
// silently and never worked around by relaxing a limit.
// The size-0 entry controls bundle evaluation and an empty tree. Other fixture
// hosts are removed before injection: they share `items`, so their hydration
// cost would otherwise grow with the target rather than cancel out.
const SSR_CANDIDATES: Scenario[] = [
  { shape: 'wide', size: 0 },
  { shape: 'deep', size: 16 },
  { shape: 'deep', size: 32 },
  { shape: 'deep', size: 48 },
  { shape: 'deep', size: 62 },
  { shape: 'deep', size: 64 },
  { shape: 'deep', size: 128 },
  { shape: 'deep', size: 240 },
  { shape: 'wide', size: 100 },
  { shape: 'wide', size: 1000 },
];

async function retainedHeap(session: CDPSession): Promise<number> {
  await session.send('HeapProfiler.collectGarbage');
  const usage = await session.send('Runtime.getHeapUsage');
  return usage.usedSize;
}

function percentile(values: number[], fraction: number): number {
  const sorted = values.toSorted((left, right) => left - right);
  return sorted[Math.max(0, Math.ceil(sorted.length * fraction) - 1)];
}

function summarize<T>(samples: T[], keys: ReadonlyArray<keyof T & string>): Record<string, unknown> {
  return Object.fromEntries(keys.map(key => {
    const values = samples.flatMap(sample => {
      const value = sample[key] as unknown;
      return typeof value === 'number' && Number.isFinite(value) ? [value] : [];
    });
    return [key, values.length ? { median: percentile(values, 0.5), p95: percentile(values, 0.95) } : null];
  }));
}

interface HeapTrend {
  rounds: number;
  slopeBytesPerRound: number;
  firstHalfMedian: number;
  secondHalfMedian: number;
  growthBytes: number;
  roundsIncreasing: number;
}

/**
 * Least-squares slope of retained heap across rounds. Reported, never gated:
 * forced GC leaves real noise, so a positive slope is a lead to investigate,
 * not a failure.
 */
function heapTrend(values: Array<number | null>): HeapTrend | null {
  const points = values.flatMap((value, index) => (value === null ? [] : [{ index, value }]));
  if (points.length < 4) return null;
  const count = points.length;
  let meanIndex = 0;
  let meanValue = 0;
  for (const point of points) {
    meanIndex += point.index / count;
    meanValue += point.value / count;
  }
  let covariance = 0;
  let variance = 0;
  let roundsIncreasing = 0;
  for (let index = 0; index < count; index++) {
    const point = points[index];
    covariance += (point.index - meanIndex) * (point.value - meanValue);
    variance += (point.index - meanIndex) ** 2;
    if (index > 0 && point.value > points[index - 1].value) roundsIncreasing++;
  }
  const half = Math.floor(count / 2);
  const firstHalf = points.slice(0, half).map(point => point.value);
  const secondHalf = points.slice(count - half).map(point => point.value);
  const firstHalfMedian = percentile(firstHalf, 0.5);
  const secondHalfMedian = percentile(secondHalf, 0.5);
  return {
    rounds: count,
    slopeBytesPerRound: variance === 0 ? 0 : covariance / variance,
    firstHalfMedian,
    secondHalfMedian,
    growthBytes: secondHalfMedian - firstHalfMedian,
    roundsIncreasing,
  };
}

function readRuns(name: string, fallback: number): number {
  const runs = Number.parseInt(process.env[name] ?? String(fallback), 10);
  expect(Number.isInteger(runs) && runs > 0, `${name} must be a positive integer`).toBe(true);
  return runs;
}

function collectErrors(page: import('@playwright/test').Page, errors: string[]): void {
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', message => {
    if (message.type() === 'error') errors.push(message.text());
  });
}

test('measures client creation of recursive-fragment trees', async ({ page }, testInfo) => {
  test.skip(process.env.WEBUI_FRAGMENT_PERF !== '1', 'Opt-in benchmark; run alone with --workers=1');
  test.setTimeout(600_000);
  expect(testInfo.config.workers, 'performance runs must be serialized').toBe(1);
  const runs = readRuns('WEBUI_FRAGMENT_PERF_RUNS', 20);
  const errors: string[] = [];
  collectErrors(page, errors);
  await page.goto(`/${FIXTURE}/fixture.html`);
  await page.waitForFunction(() =>
    (document.querySelector('test-recursive-tree') as unknown as { $ready: boolean })?.$ready,
  );
  const session = await page.context().newCDPSession(page);
  const scenarios = CLIENT_SCENARIOS;
  const samples = scenarios.map(() => [] as Array<Sample & { retainedAfterCleanupBytes: number | null }>);
  try {
    for (let round = -1; round < runs; round++) {
      for (let offset = 0; offset < scenarios.length; offset++) {
        const index = (offset + Math.max(round, 0)) % scenarios.length;
        const scenario = scenarios[index];
        const before = await retainedHeap(session);
        const sample = await page.evaluate(runScenario, scenario);
        const after = await retainedHeap(session);
        expect(sample.mountedItems).toBe(scenario.size);
        expect(sample.reinsertedItems).toBe(scenario.size);
        expect(sample.removedItems).toBe(0);
        expect(sample.hydrations).toBe(1);
        expect(sample.retainedIdentity, 'root replacement/reorder preserve keyed nodes').toBe(true);
        expect(sample.structuralEditsValid, 'mid-list/mid-depth edits preserve order and identity').toBe(true);
        expect(sample.leafUpdated).toBe(true);
        expect(sample.ownerUpdated).toBe(true);
        expect(sample.noOpMutations, 'unchanged state must not mutate the DOM').toBe(0);
        expect(
          sample.markupParity,
          `rebuilding from empty must match incremental replacement markup ` +
          `(delta ${sample.markupParityDeltaBytes} bytes)`,
        ).toBe(true);
        expect(sample.teardownSettled, 'disconnect must settle: $ready false and instance released').toBe(true);
        expect(errors, 'no browser execution errors').toEqual([]);
        if (round >= 0) {
          samples[index].push({
            ...sample,
            retainedAfterCleanupBytes: before === null || after === null ? null : after - before,
          });
        }
      }
    }
  } finally {
    await session.detach();
  }
  const timingKeys = [
    'mountMs', 'leafUpdateMs', 'ownerUpdateMs', 'replaceRootMs', 'noOpMs',
    'reorderMs', 'insertionMs', 'deepReorderMs', 'removeMs', 'reinsertMs',
    'disconnectSyncMs', 'disconnectSettleMs',
  ] as const;
  const rows = scenarios.map((scenario, index) => {
    const rounds = samples[index];
    const htmlBytes = rounds.length ? rounds[0].htmlBytes : 0;
    const timing = summarize(rounds, timingKeys);
    const mountValues = rounds.map(sample => sample.mountMs);
    return {
      ...scenario,
      runs,
      htmlBytes,
      bytesPerItem: scenario.size ? htmlBytes / scenario.size : 0,
      nsPerItemAtMedianMount: scenario.size
        ? (percentile(mountValues, 0.5) * 1e6) / scenario.size
        : 0,
      timing,
      retainedTrend: heapTrend(rounds.map(sample => sample.retainedAfterCleanupBytes)),
      retainedAfterCleanupBytes: summarize(rounds, ['retainedAfterCleanupBytes'] as const),
      observedHeapDeltaBytes: summarize(rounds, ['observedHeapDeltaBytes'] as const),
      samples: rounds,
    };
  });
  const report = {
    benchmark: 'recursive-fragments-client-creation',
    path: 'client creation (document.createElement); SSR adoption is a separate test',
    method: 'one discarded warmup, rotating scenario order; state setup and DOM instrumentation outside timers',
    memory: 'observed JS heap includes fixture data/instrumentation; retained delta measured after cleanup and forced GC',
    invariants: [
      'markupParity: remove-then-reinsert reproduces the incremental replacement markup byte for byte',
      'structuralEditsValid: insert at the middle of a wide list or half-depth of a chain, '
        + 'reverse deep siblings, then remove the new item without replacing existing keyed nodes',
      'teardownSettled: disconnect drains its microtask and releases the instance',
      'retainedTrend: reported slope of retained bytes per round, never gated',
    ],
    rows,
  };
  console.log(JSON.stringify({ ...report, rows: rows.map(({ samples: _s, ...row }) => row) }, null, 2));
  await testInfo.attach('recursive-fragments-performance', {
    body: JSON.stringify(report, null, 2),
    contentType: 'application/json',
  });
});

test('measures real SSR adoption of recursive-fragment trees', async ({ page, request }, testInfo) => {
  test.skip(process.env.WEBUI_FRAGMENT_PERF !== '1', 'Opt-in benchmark; run alone with --workers=1');
  test.setTimeout(600_000);
  expect(testInfo.config.workers, 'performance runs must be serialized').toBe(1);
  const runs = readRuns('WEBUI_FRAGMENT_PERF_SSR_RUNS', readRuns('WEBUI_FRAGMENT_PERF_RUNS', 20));
  expect(
    existsSync(projectionManifest),
    `${projectionManifest} is missing; start the fixture server so SSR uses the same projection metadata`,
  ).toBe(true);
  const baseState = JSON.parse(readFileSync(defaultState, 'utf-8')) as Record<string, unknown>;
  const { renderFixtures } = await import('@microsoft/webui-test-support/fixture-render');

  const renders: SsrRender[] = [];
  const unsupported: Array<Scenario & { error: string }> = [];
  for (const scenario of SSR_CANDIDATES) {
    try {
      renders.push(renderSsr(scenario, baseState, renderFixtures));
    } catch (error) {
      if (scenario.shape !== 'deep' || scenario.size <= 16 || !(error instanceof Error)
        || !error.message.startsWith('State JSON error: recursion limit exceeded')) {
        throw error;
      }
      unsupported.push({ ...scenario, error: error.message });
    }
  }
  for (const required of ['deep/16', 'wide/0', 'wide/100', 'wide/1000']) {
    expect(renders.some(({ scenario }) => `${scenario.shape}/${scenario.size}` === required),
      `required SSR scenario ${required} must render`).toBe(true);
  }

  const bundleResponse = await request.get(BUNDLE_PATH);
  expect(bundleResponse.ok(), `${BUNDLE_PATH} must be served by the fixture server`).toBe(true);
  const source = await bundleResponse.text();
  expect(source.length, 'client bundle must not be empty').toBeGreaterThan(0);

  const errors: string[] = [];
  collectErrors(page, errors);
  let body = '';
  await page.route(`**${SSR_ROUTE}`, route => route.fulfill({
    status: 200,
    contentType: 'text/html; charset=utf-8',
    headers: { 'cache-control': 'no-store' },
    body,
  }));

  const session = await page.context().newCDPSession(page);
  const samples = renders.map(() => [] as Array<AdoptSample & { retainedAfterAdoptionBytes: number }>);
  try {
    for (let round = -1; round < runs; round++) {
      for (let offset = 0; offset < renders.length; offset++) {
        const index = (offset + Math.max(round, 0)) % renders.length;
        const render = renders[index];
        body = render.html;
        await page.goto(SSR_ROUTE);
        const removedSiblings = await page.evaluate(() => {
          const siblings = document.querySelectorAll(
            'test-recursive-light,test-recursive-table,test-recursive-unknown,test-recursive-lazy',
          );
          for (const sibling of siblings) sibling.remove();
          return siblings.length;
        });
        expect(removedSiblings, 'remove all size-dependent sibling hosts before timing').toBe(4);
        const before = await retainedHeap(session);
        const sample = await page.evaluate(adoptScenario, {
          tag: TAG,
          source,
          lastId: `node-${render.scenario.size - 1}`,
        });
        const after = await retainedHeap(session);
        expect(sample.definedBeforeInjection, 'adoption must be timed on an undefined tag').toBe(false);
        expect(sample.ssrItems, 'server-rendered markup must already hold every item').toBe(render.scenario.size);
        expect(sample.adoptedItems, 'adoption must not add or drop items').toBe(render.scenario.size);
        expect(sample.sameHostObject, 'adoption must upgrade the parsed host in place').toBe(true);
        expect(sample.adoptedIdentity, 'adoption must keep the server-rendered nodes').toBe(true);
        expect(sample.hydrations, 'each host hydrates exactly once').toBe(1);
        expect(sample.ready).toBe(true);
        expect(sample.siblingHosts, 'only the target tree may hydrate').toBe(0);
        if (render.scenario.size > 0) {
          expect(sample.leafUpdated, 'adopted tree must be live, not inert markup').toBe(true);
        }
        expect(errors, 'no browser execution errors').toEqual([]);
        if (round >= 0) {
          samples[index].push({
            ...sample,
            retainedAfterAdoptionBytes: after - before,
          });
        }
      }
    }
  } finally {
    await session.detach();
    await page.unroute(`**${SSR_ROUTE}`);
  }

  const timingKeys = ['bundleEvalMs', 'adoptSettleMs', 'adoptTotalMs', 'hydratedLeafUpdateMs'] as const;
  const controlIndex = renders.findIndex(render => render.scenario.size === 0);
  const controlTotals = controlIndex === -1 ? [] : samples[controlIndex].map(sample => sample.adoptTotalMs);
  const controlMedianMs = controlTotals.length ? percentile(controlTotals, 0.5) : null;
  const rows = renders.map((render, index) => {
    const rounds = samples[index];
    const ssrHtmlBytes = rounds.length ? rounds[0].ssrHtmlBytes : 0;
    const totals = rounds.map(sample => sample.adoptTotalMs);
    const medianTotal = totals.length ? percentile(totals, 0.5) : null;
    const treeOnlyMs = medianTotal === null || controlMedianMs === null
      ? null
      : medianTotal - controlMedianMs;
    return {
      ...render.scenario,
      control: render.scenario.size === 0,
      runs,
      ssrHtmlBytes,
      ssrDocumentBytes: render.bytes,
      bytesPerItem: render.scenario.size ? ssrHtmlBytes / render.scenario.size : 0,
      adoptTotalMinusControlMs: treeOnlyMs,
      nsPerItemAdoptedAboveControl: render.scenario.size && treeOnlyMs !== null
        ? (treeOnlyMs * 1e6) / render.scenario.size
        : 0,
      timing: summarize(rounds, timingKeys),
      markupUnchanged: rounds.every(sample => sample.markupUnchanged),
      markupDeltaBytes: rounds.length ? rounds[0].markupDeltaBytes : 0,
      identityProbes: rounds.length ? rounds[0].identityProbes : 0,
      retainedTrend: heapTrend(rounds.map(sample => sample.retainedAfterAdoptionBytes)),
      retainedAfterAdoptionBytes: summarize(rounds, ['retainedAfterAdoptionBytes'] as const),
      observedHeapDeltaBytes: summarize(rounds, ['observedHeapDeltaBytes'] as const),
      samples: rounds,
    };
  });
  const report = {
    benchmark: 'recursive-fragments-ssr-adoption',
    path: 'real SSR HTML parsed with no framework present, then the current bundle injected',
    method: 'SSR rendered through the native pipeline per scenario outside all timers; '
      + 'entry tag stripped so the parsed DOM is framework-free; '
      + 'one discarded warmup, rotating scenario order; fresh document per round',
    control: controlMedianMs === null
      ? 'no control round completed'
      : `size-0 scenario median ${controlMedianMs.toFixed(3)}ms covers bundle compile, define and the `
        + 'empty target; all sibling hosts are removed before injection',
    memory: 'observed JS heap includes bundle source and instrumentation; retained delta is measured '
      + 'after adoption and forced GC with the target still mounted, not after teardown',
    invariants: [
      'definedBeforeInjection false: the tag is undefined until the timer starts',
      'sameHostObject/adoptedIdentity: the parsed host and its item nodes are upgraded in place, not rebuilt',
      'leafUpdated: the adopted tree reacts to hydrated JS state',
    ],
    unsupported: unsupported.map(entry => ({
      ...entry,
      note: 'native state JSON recursion limit; other render failures fail the benchmark',
    })),
    rows,
  };
  console.log(JSON.stringify({ ...report, rows: rows.map(({ samples: _s, ...row }) => row) }, null, 2));
  await testInfo.attach('recursive-fragments-ssr-adoption', {
    body: JSON.stringify(report, null, 2),
    contentType: 'application/json',
  });
});
