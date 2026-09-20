// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import {
  expect,
  test,
  type Browser,
  type CDPSession,
} from '@playwright/test';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  buildLazyFixture,
  ITEM_COUNTS,
  lazyBaseHtml,
  todoRootsHtml,
  type LazyFixture,
} from './lib/lazy-fixtures.js';
import {
  insertLazyRoots,
  runLazyHydration,
  type LazyMode,
  type LazyRenderMetrics,
  type LazyRunMetrics,
} from './lib/lazy-driver.js';
import { median, medianNullable, p95 } from './lib/stats.js';

const here = dirname(fileURLToPath(import.meta.url));
const BASELINE_DIR = resolve(here, '..', '..', '..', '..', 'target', 'bench-baselines');
const ENV_RUNS = Number.parseInt(process.env.WEBUI_LAZY_HYDRATION_RUNS ?? '', 10);
const RUNS = Number.isFinite(ENV_RUNS) && ENV_RUNS > 0 ? ENV_RUNS : 15;
const ENV_TRACE_RUNS = Number.parseInt(
  process.env.WEBUI_LAZY_HYDRATION_TRACE_RUNS ?? '',
  10,
);
const TRACE_RUNS = Number.isFinite(ENV_TRACE_RUNS) && ENV_TRACE_RUNS >= 0
  ? ENV_TRACE_RUNS
  : 3;
const ALL_MODES: readonly LazyMode[] = ['eager', 'lazy-hydrate', 'lazy-render'];
const MODES = selectedModes(process.env.WEBUI_LAZY_HYDRATION_MODES);

function selectedModes(value: string | undefined): readonly LazyMode[] {
  if (!value) return ALL_MODES;
  const requested = value.split(',');
  const modes: LazyMode[] = [];
  for (let i = 0; i < ALL_MODES.length; i++) {
    if (requested.includes(ALL_MODES[i])) modes.push(ALL_MODES[i]);
  }
  if (modes.length !== requested.length || modes.length === 0) {
    throw new Error(
      `WEBUI_LAZY_HYDRATION_MODES must contain only: ${ALL_MODES.join(', ')}`,
    );
  }
  return modes;
}

interface ChromiumRenderMetrics {
  recalcStyleMs: number;
  layoutMs: number;
  scriptMs: number;
  taskMs: number;
  recalcStyleCount: number;
  layoutCount: number;
  nodeCount: number;
  layoutObjectCount: number;
}

interface TraceMetrics {
  parseHtmlMs: number;
  styleMs: number;
  layoutMs: number;
  prePaintMs: number;
  paintMs: number;
  rasterMs: number;
}

interface RunResult {
  metrics: LazyRunMetrics;
  render: LazyRenderMetrics;
  chromium: ChromiumRenderMetrics;
  retained: number | null;
  totalRetained: number | null;
  trace?: TraceMetrics;
}

interface Aggregate {
  mode: LazyMode;
  itemCount: number;
  domConstructionMsMedian: number;
  forcedStyleLayoutMsMedian: number;
  presentationMsMedian: number;
  initialRenderReadyMsMedian: number;
  recalcStyleMsMedian: number;
  layoutMsMedian: number;
  scriptMsMedian: number;
  taskMsMedian: number;
  recalcStyleCountMedian: number;
  layoutCountMedian: number;
  nodeCountMedian: number;
  layoutObjectCountMedian: number;
  documentHeightMedian: number;
  traceSamples: number;
  traceParseHtmlMsMedian: number | null;
  traceStyleMsMedian: number | null;
  traceLayoutMsMedian: number | null;
  tracePrePaintMsMedian: number | null;
  tracePaintMsMedian: number | null;
  traceRasterMsMedian: number | null;
  bundleInitMsMedian: number;
  hydrationCpuMsMedian: number;
  hydrationCpuMsP95: number;
  definitionMsMedian: number;
  initialReadyMsMedian: number;
  initialReadyMsP95: number;
  initialHydratedCountMedian: number;
  initialListenerCountMedian: number;
  initialPeakHeapDeltaBytesMedian: number | null;
  retainedHeapDeltaBytesMedian: number | null;
  totalRetainedHeapDeltaBytesMedian: number | null;
  maxLongTaskMsMedian: number | null;
  visibleInteractionMsMedian: number;
  dormantInteractionMsMedian: number;
}

interface BundleSizes {
  minifiedBytes: number;
  gzipBytes: number;
}

interface Snapshot {
  runs: number;
  traceRuns: number;
  bundles: Partial<Record<LazyMode, BundleSizes>>;
  rows: Aggregate[];
}

async function heapUsage(session: CDPSession): Promise<number | null> {
  try {
    await session.send('HeapProfiler.collectGarbage');
    const usage = await session.send('Runtime.getHeapUsage') as { usedSize: number };
    return usage.usedSize;
  } catch {
    return null;
  }
}

interface PerformanceMetric {
  name: string;
  value: number;
}

interface TraceCapture {
  completed: Promise<{ stream?: string }>;
}

async function performanceSnapshot(
  session: CDPSession,
): Promise<PerformanceMetric[]> {
  const result = await session.send('Performance.getMetrics') as {
    metrics: PerformanceMetric[];
  };
  return result.metrics;
}

function metricValue(metrics: readonly PerformanceMetric[], name: string): number {
  for (let i = 0; i < metrics.length; i++) {
    if (metrics[i].name === name) return metrics[i].value;
  }
  throw new Error(`Chromium performance metric "${name}" is unavailable`);
}

function chromiumRenderMetrics(
  before: readonly PerformanceMetric[],
  after: readonly PerformanceMetric[],
): ChromiumRenderMetrics {
  const delta = (name: string): number =>
    metricValue(after, name) - metricValue(before, name);
  return {
    recalcStyleMs: delta('RecalcStyleDuration') * 1_000,
    layoutMs: delta('LayoutDuration') * 1_000,
    scriptMs: delta('ScriptDuration') * 1_000,
    taskMs: delta('TaskDuration') * 1_000,
    recalcStyleCount: delta('RecalcStyleCount'),
    layoutCount: delta('LayoutCount'),
    nodeCount: delta('Nodes'),
    layoutObjectCount: delta('LayoutObjects'),
  };
}

async function startTrace(session: CDPSession): Promise<TraceCapture> {
  let complete: ((event: { stream?: string }) => void) | undefined;
  const completed = new Promise<{ stream?: string }>((resolve) => {
    complete = resolve;
  });
  session.once('Tracing.tracingComplete', (event) => complete?.(event));
  await session.send('Tracing.start', {
    categories:
      'devtools.timeline,disabled-by-default-devtools.timeline',
    transferMode: 'ReturnAsStream',
  });
  return { completed };
}

async function stopTrace(
  session: CDPSession,
  capture: TraceCapture,
): Promise<TraceMetrics> {
  await session.send('Tracing.end');
  const completed = await capture.completed;
  if (!completed.stream) {
    throw new Error('Chromium tracing completed without a readable stream');
  }

  let json = '';
  while (true) {
    const chunk = await session.send('IO.read', {
      handle: completed.stream,
    }) as {
      data: string;
      base64Encoded?: boolean;
      eof?: boolean;
    };
    json += chunk.base64Encoded
      ? Buffer.from(chunk.data, 'base64').toString('utf8')
      : chunk.data;
    if (chunk.eof) break;
  }
  await session.send('IO.close', { handle: completed.stream });

  const trace = JSON.parse(json) as {
    traceEvents?: Array<{ name?: string; dur?: number }>;
  };
  const metrics: TraceMetrics = {
    parseHtmlMs: 0,
    styleMs: 0,
    layoutMs: 0,
    prePaintMs: 0,
    paintMs: 0,
    rasterMs: 0,
  };
  const events = trace.traceEvents ?? [];
  for (let i = 0; i < events.length; i++) {
    const event = events[i];
    if (typeof event.dur !== 'number') continue;
    const durationMs = event.dur / 1_000;
    switch (event.name) {
      case 'ParseHTML':
        metrics.parseHtmlMs += durationMs;
        break;
      case 'UpdateLayoutTree':
        metrics.styleMs += durationMs;
        break;
      case 'Layout':
        metrics.layoutMs += durationMs;
        break;
      case 'PrePaint':
        metrics.prePaintMs += durationMs;
        break;
      case 'Paint':
        metrics.paintMs += durationMs;
        break;
      case 'RasterTask':
        metrics.rasterMs += durationMs;
        break;
    }
  }
  return metrics;
}

async function runOnce(
  browser: Browser,
  fixture: LazyFixture,
  mode: LazyMode,
  itemCount: number,
  collectTrace = false,
): Promise<RunResult> {
  const page = await browser.newPage();
  const errors: string[] = [];
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text());
  });
  page.on('pageerror', (error) => errors.push(error.message));
  const session = await page.context().newCDPSession(page);
  let traceCapture: TraceCapture | undefined;
  try {
    await page.setContent(lazyBaseHtml(mode), { waitUntil: 'load' });
    await session.send('Performance.enable');
    const totalBefore = await heapUsage(session);
    await page.evaluate((code) => {
      const started = performance.now();
      (0, eval)(code);
      (
        window as unknown as {
          __benchBundleInitMs?: number;
        }
      ).__benchBundleInitMs = performance.now() - started;
    }, fixture.code);
    if (collectTrace) traceCapture = await startTrace(session);
    const chromiumBefore = await performanceSnapshot(session);
    const render = await page.evaluate(insertLazyRoots, todoRootsHtml(itemCount));
    const chromiumAfter = await performanceSnapshot(session);
    const trace = traceCapture
      ? await stopTrace(session, traceCapture)
      : undefined;
    traceCapture = undefined;
    const before = await heapUsage(session);
    const metrics = await page.evaluate(runLazyHydration, mode);
    const after = await heapUsage(session);

    expect(errors, 'no browser errors').toEqual([]);
    expect(metrics.liveRootCount).toBe(itemCount);
    expect(metrics.interactionCount).toBe(2);
    expect(render.documentHeight).toBe(Math.max(720, itemCount * 72));
    if (mode === 'eager') {
      expect(metrics.initialHydratedCount).toBe(itemCount);
      expect(metrics.dormantInteractionHydrated).toBe(false);
    } else if (itemCount > 10) {
      expect(metrics.initialHydratedCount).toBeGreaterThan(0);
      expect(metrics.initialHydratedCount).toBeLessThan(itemCount);
      expect(metrics.dormantInteractionHydrated).toBe(true);
    } else {
      expect(metrics.initialHydratedCount).toBe(itemCount);
    }
    expect(metrics.dormantContentSkipped).toBe(
      mode === 'lazy-render' && itemCount > 10,
    );

    return {
      metrics,
      render,
      chromium: chromiumRenderMetrics(chromiumBefore, chromiumAfter),
      retained: before === null || after === null ? null : after - before,
      totalRetained:
        totalBefore === null || after === null ? null : after - totalBefore,
      trace,
    };
  } finally {
    if (traceCapture) {
      try {
        await stopTrace(session, traceCapture);
      } catch (error) {
        console.error('failed to stop Chromium trace after benchmark error', error);
      }
    }
    await session.detach().catch(() => {});
    await page.close();
  }
}

function aggregate(
  mode: LazyMode,
  itemCount: number,
  runs: RunResult[],
  traces: TraceMetrics[],
): Aggregate {
  const values = (key: keyof LazyRunMetrics): number[] =>
    runs.map((run) => run.metrics[key]).filter((value): value is number =>
      typeof value === 'number'
    );
  const renderValues = (key: keyof LazyRenderMetrics): number[] =>
    runs.map((run) => run.render[key]);
  const chromiumValues = (key: keyof ChromiumRenderMetrics): number[] =>
    runs.map((run) => run.chromium[key]);
  const traceValues = (key: keyof TraceMetrics): number[] =>
    traces.map((trace) => trace[key]);
  return {
    mode,
    itemCount,
    domConstructionMsMedian: median(renderValues('domConstructionMs')),
    forcedStyleLayoutMsMedian: median(renderValues('forcedStyleLayoutMs')),
    presentationMsMedian: median(renderValues('presentationMs')),
    initialRenderReadyMsMedian: median(renderValues('initialRenderReadyMs')),
    recalcStyleMsMedian: median(chromiumValues('recalcStyleMs')),
    layoutMsMedian: median(chromiumValues('layoutMs')),
    scriptMsMedian: median(chromiumValues('scriptMs')),
    taskMsMedian: median(chromiumValues('taskMs')),
    recalcStyleCountMedian: median(chromiumValues('recalcStyleCount')),
    layoutCountMedian: median(chromiumValues('layoutCount')),
    nodeCountMedian: median(chromiumValues('nodeCount')),
    layoutObjectCountMedian: median(chromiumValues('layoutObjectCount')),
    documentHeightMedian: median(renderValues('documentHeight')),
    traceSamples: traces.length,
    traceParseHtmlMsMedian:
      traces.length === 0 ? null : median(traceValues('parseHtmlMs')),
    traceStyleMsMedian:
      traces.length === 0 ? null : median(traceValues('styleMs')),
    traceLayoutMsMedian:
      traces.length === 0 ? null : median(traceValues('layoutMs')),
    tracePrePaintMsMedian:
      traces.length === 0 ? null : median(traceValues('prePaintMs')),
    tracePaintMsMedian:
      traces.length === 0 ? null : median(traceValues('paintMs')),
    traceRasterMsMedian:
      traces.length === 0 ? null : median(traceValues('rasterMs')),
    bundleInitMsMedian: median(values('bundleInitMs')),
    hydrationCpuMsMedian: median(values('hydrationCpuMs')),
    hydrationCpuMsP95: p95(values('hydrationCpuMs')),
    definitionMsMedian: median(values('definitionMs')),
    initialReadyMsMedian: median(values('initialReadyMs')),
    initialReadyMsP95: p95(values('initialReadyMs')),
    initialHydratedCountMedian: median(values('initialHydratedCount')),
    initialListenerCountMedian: median(values('initialListenerCount')),
    initialPeakHeapDeltaBytesMedian: medianNullable(
      runs.map((run) => run.metrics.initialPeakHeapDeltaBytes),
    ),
    retainedHeapDeltaBytesMedian: medianNullable(runs.map((run) => run.retained)),
    totalRetainedHeapDeltaBytesMedian: medianNullable(
      runs.map((run) => run.totalRetained),
    ),
    maxLongTaskMsMedian: medianNullable(runs.map((run) => run.metrics.maxLongTaskMs)),
    visibleInteractionMsMedian: median(values('visibleInteractionMs')),
    dormantInteractionMsMedian: median(values('dormantInteractionMs')),
  };
}

function formatMs(value: number | null): string {
  return value === null ? 'n/a' : value.toFixed(2);
}

function formatBytes(value: number | null): string {
  if (value === null) return 'n/a';
  return `${(value / 1024).toFixed(1)}KiB`;
}

async function saveSnapshot(name: string, snapshot: Snapshot): Promise<void> {
  await mkdir(BASELINE_DIR, { recursive: true });
  await writeFile(
    resolve(BASELINE_DIR, `browser-lazy-hydration-${name}.json`),
    `${JSON.stringify(snapshot, null, 2)}\n`,
    'utf8',
  );
}

async function compareSnapshot(name: string, current: Snapshot): Promise<void> {
  const path = resolve(BASELINE_DIR, `browser-lazy-hydration-${name}.json`);
  const before = JSON.parse(await readFile(path, 'utf8')) as Snapshot;
  console.log(`\nLazy hydration delta vs "${name}":`);
  for (const mode of Object.keys(current.bundles) as LazyMode[]) {
    const currentBundle = current.bundles[mode];
    const baselineBundle = before.bundles?.[mode];
    if (!currentBundle || !baselineBundle) continue;
    console.log(
      `  ${mode} bundle minified: ${currentBundle.minifiedBytes - baselineBundle.minifiedBytes} B`,
    );
    console.log(
      `  ${mode} bundle gzip:     ${currentBundle.gzipBytes - baselineBundle.gzipBytes} B`,
    );
  }
  for (const row of current.rows) {
    const baseline = before.rows.find((candidate) =>
      candidate.mode === row.mode && candidate.itemCount === row.itemCount
    );
    if (!baseline) continue;
    const cpuPct = baseline.hydrationCpuMsMedian === 0
      ? 0
      : ((row.hydrationCpuMsMedian / baseline.hydrationCpuMsMedian) - 1) * 100;
    console.log(
      `  ${row.mode} ${row.itemCount}: hydration CPU ${cpuPct >= 0 ? '+' : ''}${cpuPct.toFixed(1)}%`,
    );
  }
}

test('component work reduction 10/100/1000 item performance matrix', async ({ browser }) => {
  const fixtures = new Map<LazyMode, LazyFixture>();
  for (const mode of MODES) {
    fixtures.set(mode, await buildLazyFixture(mode));
  }

  // Item 3 / architecture guidance: the eager bundle must never reach the
  // viewport/interaction coordinator. Assert this from the actual esbuild
  // module graph rather than inferring it from bundle size.
  const eagerFixture = fixtures.get('eager');
  if (eagerFixture) {
    const coordinatorInputs = eagerFixture.inputs.filter((input) =>
      input.endsWith('lazy-hydration-coordinator.ts')
    );
    expect(
      coordinatorInputs,
      'the eager bundle must not reach lazy-hydration-coordinator.ts',
    ).toEqual([]);
  }
  for (const mode of ['lazy-hydrate', 'lazy-render'] as const) {
    const fixture = fixtures.get(mode);
    if (!fixture) continue;
    const coordinatorInputs = fixture.inputs.filter((input) =>
      input.endsWith('lazy-hydration-coordinator.ts')
    );
    expect(
      coordinatorInputs,
      `the ${mode} bundle must reach lazy-hydration-coordinator.ts`,
    ).not.toEqual([]);
  }

  const samples = new Map<string, RunResult[]>();

  for (let round = 0; round < RUNS; round++) {
    const itemCounts = round % 2 === 0
      ? ITEM_COUNTS
      : [...ITEM_COUNTS].reverse();
    const modes = round % 2 === 0 ? MODES : [...MODES].reverse();
    for (const itemCount of itemCounts) {
      for (const mode of modes) {
        const fixture = fixtures.get(mode);
        if (!fixture) continue;
        const key = `${mode}:${itemCount}`;
        const bucket = samples.get(key) ?? [];
        bucket.push(await runOnce(browser, fixture, mode, itemCount));
        samples.set(key, bucket);
      }
    }
  }

  const traceSamples = new Map<string, TraceMetrics[]>();
  for (let round = 0; round < TRACE_RUNS; round++) {
    const itemCounts = round % 2 === 0
      ? ITEM_COUNTS
      : [...ITEM_COUNTS].reverse();
    const modes = round % 2 === 0 ? MODES : [...MODES].reverse();
    for (const itemCount of itemCounts) {
      for (const mode of modes) {
        const fixture = fixtures.get(mode);
        if (!fixture) continue;
        const key = `${mode}:${itemCount}`;
        const result = await runOnce(
          browser,
          fixture,
          mode,
          itemCount,
          true,
        );
        if (!result.trace) {
          throw new Error(`trace metrics missing for ${key}`);
        }
        const bucket = traceSamples.get(key) ?? [];
        bucket.push(result.trace);
        traceSamples.set(key, bucket);
      }
    }
  }

  const rows: Aggregate[] = [];
  for (const itemCount of ITEM_COUNTS) {
    for (const mode of MODES) {
      const key = `${mode}:${itemCount}`;
      const runs = samples.get(key);
      if (runs) {
        rows.push(aggregate(
          mode,
          itemCount,
          runs,
          traceSamples.get(key) ?? [],
        ));
      }
    }
  }
  const bundles: Partial<Record<LazyMode, BundleSizes>> = {};
  for (const [mode, fixture] of fixtures) {
    bundles[mode] = {
      minifiedBytes: fixture.minifiedBytes,
      gzipBytes: fixture.gzipBytes,
    };
  }
  const snapshot: Snapshot = {
    runs: RUNS,
    traceRuns: TRACE_RUNS,
    bundles,
    rows,
  };

  console.log('\nOffscreen work-reduction browser matrix');
  for (const [mode, fixture] of fixtures) {
    console.log(
      `  ${mode} bundle: ${fixture.minifiedBytes} B min / ${fixture.gzipBytes} B gzip`,
    );
  }
  console.log('\n  Hydration and memory');
  console.log('  mode      | items | bundle init | hydration CPU med/p95 | define | initial ready med/p95 | hydrated | listeners | peak heap | hydration retained | total retained | long task | visible/dormant click');
  for (const row of rows) {
    console.log(
      `  ${row.mode.padEnd(9)} | ${String(row.itemCount).padStart(5)} | `
      + `${formatMs(row.bundleInitMsMedian)} ms | `
      + `${formatMs(row.hydrationCpuMsMedian)}/${formatMs(row.hydrationCpuMsP95)} ms | `
      + `${formatMs(row.definitionMsMedian)} ms | `
      + `${formatMs(row.initialReadyMsMedian)}/${formatMs(row.initialReadyMsP95)} ms | `
      + `${String(row.initialHydratedCountMedian).padStart(8)} | `
      + `${String(row.initialListenerCountMedian).padStart(9)} | `
      + `${formatBytes(row.initialPeakHeapDeltaBytesMedian).padStart(9)} | `
      + `${formatBytes(row.retainedHeapDeltaBytesMedian).padStart(18)} | `
      + `${formatBytes(row.totalRetainedHeapDeltaBytesMedian).padStart(14)} | `
      + `${formatMs(row.maxLongTaskMsMedian).padStart(8)} | `
      + `${formatMs(row.visibleInteractionMsMedian)}/${formatMs(row.dormantInteractionMsMedian)} ms`,
    );
  }
  console.log(`\n  Rendering pipeline (${TRACE_RUNS} trace samples per cell)`);
  console.log('  mode      | items | DOM construct | forced style/layout | present | render ready | CDP style/layout | trace parse/style/layout | prepaint/paint/raster | nodes/layout objects');
  for (const row of rows) {
    console.log(
      `  ${row.mode.padEnd(9)} | ${String(row.itemCount).padStart(5)} | `
      + `${formatMs(row.domConstructionMsMedian)} ms | `
      + `${formatMs(row.forcedStyleLayoutMsMedian)} ms | `
      + `${formatMs(row.presentationMsMedian)} ms | `
      + `${formatMs(row.initialRenderReadyMsMedian)} ms | `
      + `${formatMs(row.recalcStyleMsMedian)}/${formatMs(row.layoutMsMedian)} ms | `
      + `${formatMs(row.traceParseHtmlMsMedian)}/${formatMs(row.traceStyleMsMedian)}/${formatMs(row.traceLayoutMsMedian)} ms | `
      + `${formatMs(row.tracePrePaintMsMedian)}/${formatMs(row.tracePaintMsMedian)}/${formatMs(row.traceRasterMsMedian)} ms | `
      + `${row.nodeCountMedian.toFixed(0)}/${row.layoutObjectCountMedian.toFixed(0)}`,
    );
  }

  const saveName = process.env.WEBUI_LAZY_HYDRATION_SAVE;
  if (saveName) await saveSnapshot(saveName, snapshot);
  const compareName = process.env.WEBUI_LAZY_HYDRATION_COMPARE;
  if (compareName) await compareSnapshot(compareName, snapshot);
});
