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
  type LazyRunMetrics,
} from './lib/lazy-driver.js';
import { median, medianNullable, p95 } from './lib/stats.js';

const here = dirname(fileURLToPath(import.meta.url));
const BASELINE_DIR = resolve(here, '..', '..', '..', '..', 'target', 'bench-baselines');
const ENV_RUNS = Number.parseInt(process.env.WEBUI_LAZY_HYDRATION_RUNS ?? '', 10);
const RUNS = Number.isFinite(ENV_RUNS) && ENV_RUNS > 0 ? ENV_RUNS : 15;
const requestedModes = process.env.WEBUI_LAZY_HYDRATION_MODES;
const MODES: readonly LazyMode[] = requestedModes === 'eager'
  ? ['eager']
  : requestedModes === 'lazy'
    ? ['lazy']
    : ['eager', 'lazy'];

interface Aggregate {
  mode: LazyMode;
  itemCount: number;
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

interface Snapshot {
  runs: number;
  bundleMinifiedBytes: number;
  bundleGzipBytes: number;
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

async function runOnce(
  browser: Browser,
  fixture: LazyFixture,
  mode: LazyMode,
  itemCount: number,
): Promise<{
  metrics: LazyRunMetrics;
  retained: number | null;
  totalRetained: number | null;
}> {
  const page = await browser.newPage();
  const errors: string[] = [];
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text());
  });
  page.on('pageerror', (error) => errors.push(error.message));
  const session = await page.context().newCDPSession(page);
  try {
    await page.setContent(lazyBaseHtml(), { waitUntil: 'load' });
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
    await page.evaluate(insertLazyRoots, todoRootsHtml(itemCount));
    const before = await heapUsage(session);
    const metrics = await page.evaluate(runLazyHydration, mode);
    const after = await heapUsage(session);

    expect(errors, 'no browser errors').toEqual([]);
    expect(metrics.liveRootCount).toBe(itemCount);
    expect(metrics.interactionCount).toBe(2);
    if (mode === 'eager') {
      expect(metrics.initialHydratedCount).toBe(itemCount);
      expect(metrics.dormantInteractionHydrated).toBe(false);
    } else if (itemCount === 1_000) {
      expect(metrics.initialHydratedCount).toBeGreaterThan(0);
      expect(metrics.initialHydratedCount).toBeLessThan(itemCount);
      expect(metrics.dormantInteractionHydrated).toBe(true);
    } else {
      expect(metrics.initialHydratedCount).toBe(itemCount);
    }

    return {
      metrics,
      retained: before === null || after === null ? null : after - before,
      totalRetained:
        totalBefore === null || after === null ? null : after - totalBefore,
    };
  } finally {
    await session.detach().catch(() => {});
    await page.close();
  }
}

function aggregate(
  mode: LazyMode,
  itemCount: number,
  runs: Array<{
    metrics: LazyRunMetrics;
    retained: number | null;
    totalRetained: number | null;
  }>,
): Aggregate {
  const values = (key: keyof LazyRunMetrics): number[] =>
    runs.map((run) => run.metrics[key]).filter((value): value is number =>
      typeof value === 'number'
    );
  return {
    mode,
    itemCount,
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
  console.log(`  bundle minified: ${current.bundleMinifiedBytes - before.bundleMinifiedBytes} B`);
  console.log(`  bundle gzip:     ${current.bundleGzipBytes - before.bundleGzipBytes} B`);
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

test('component lazy hydration 10/1000 item performance matrix', async ({ browser }) => {
  const fixture = await buildLazyFixture();
  const samples = new Map<string, Array<{
    metrics: LazyRunMetrics;
    retained: number | null;
    totalRetained: number | null;
  }>>();

  for (let round = 0; round < RUNS; round++) {
    const itemCounts = round % 2 === 0
      ? ITEM_COUNTS
      : [...ITEM_COUNTS].reverse();
    const modes = round % 2 === 0 ? MODES : [...MODES].reverse();
    for (const itemCount of itemCounts) {
      for (const mode of modes) {
        const key = `${mode}:${itemCount}`;
        const bucket = samples.get(key) ?? [];
        bucket.push(await runOnce(browser, fixture, mode, itemCount));
        samples.set(key, bucket);
      }
    }
  }

  const rows: Aggregate[] = [];
  for (const itemCount of ITEM_COUNTS) {
    for (const mode of MODES) {
      const runs = samples.get(`${mode}:${itemCount}`);
      if (runs) rows.push(aggregate(mode, itemCount, runs));
    }
  }
  const snapshot: Snapshot = {
    runs: RUNS,
    bundleMinifiedBytes: fixture.minifiedBytes,
    bundleGzipBytes: fixture.gzipBytes,
    rows,
  };

  console.log('\nLazy hydration browser matrix');
  console.log(`  production bundle: ${fixture.minifiedBytes} B min / ${fixture.gzipBytes} B gzip`);
  console.log('  mode  | items | bundle init | hydration CPU med/p95 | define | initial ready med/p95 | hydrated | listeners | peak heap | hydration retained | total retained | long task | visible/dormant click');
  for (const row of rows) {
    console.log(
      `  ${row.mode.padEnd(5)} | ${String(row.itemCount).padStart(5)} | `
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

  const saveName = process.env.WEBUI_LAZY_HYDRATION_SAVE;
  if (saveName) await saveSnapshot(saveName, snapshot);
  const compareName = process.env.WEBUI_LAZY_HYDRATION_COMPARE;
  if (compareName) await compareSnapshot(compareName, snapshot);
});
