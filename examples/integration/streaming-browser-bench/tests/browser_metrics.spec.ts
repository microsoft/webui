// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Browser-perceived metrics: TTFB, FCP, LCP, domContentLoaded, load.
 *
 * Compares /buf (whole body in one HTTP chunk) vs /stream (chunked
 * via tokio mpsc + ReceiverStream + lock-free chunk pool) at four
 * render-cost scenarios.
 *
 * Metrics are captured via:
 *
 *   - PerformanceNavigationTiming (TTFB, domContentLoaded, load)
 *   - PerformanceObserver for `paint` (FCP)
 *   - PerformanceObserver for `largest-contentful-paint` (LCP)
 *
 * Each scenario runs N iterations; we report median + p99. Browser
 * cache is disabled per test (per playwright.config.ts) so every
 * navigation is a clean cold load.
 *
 * # Baseline workflow (before/after comparison)
 *
 *   WEBUI_BENCH_SAVE=before pnpm test     # save current numbers as 'before'
 *   …make change…
 *   WEBUI_BENCH_COMPARE=before pnpm test  # run + diff vs 'before'
 *
 * Baselines live at `target/bench-baselines/browser-<name>.json`.
 */

import { test, expect } from '@playwright/test';
import { mkdirSync, readFileSync, writeFileSync, existsSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

interface PageMetrics {
  ttfbMs: number;
  fcpMs: number;
  lcpMs: number;
  dclMs: number;
  loadMs: number;
  bodyLen: number;
}

interface SnapshotRow {
  scenario: string;
  path: 'buffered' | 'streaming';
  ttfbMsMedian: number;
  fcpMsMedian: number;
  lcpMsMedian: number;
  dclMsMedian: number;
  loadMsMedian: number;
  bodyBytes: number;
  iters: number;
}

interface Snapshot {
  schema: number;
  name: string;
  timestampUnix: number;
  rows: SnapshotRow[];
}

const SNAPSHOT_SCHEMA = 1;

const SCENARIOS = [
  { delay: 0,   label: 'no-delay (~0 ms render)' },
  { delay: 50,  label: '50µs/write (~25 ms render)' },
  { delay: 200, label: '200µs/write (~100 ms render)' },
  { delay: 500, label: '500µs/write (~250 ms render)' },
];

const ITERS = 8;

/// Install the LCP `PerformanceObserver` exactly once on the
/// browser context. Playwright's `page.addInitScript` is **cumulative**
/// — each call adds another script that runs on every subsequent
/// navigation. Calling it inside `measure` (as we did originally)
/// would register N copies of the observer over N navigations, which
/// is benchmark-skewing waste. This helper enforces the install-once
/// contract by guarding on a context-level flag.
async function ensureLcpObserverInstalled(page: import('@playwright/test').Page): Promise<void> {
  const ctx = page.context() as unknown as { __lcpObserverInstalled?: boolean };
  if (ctx.__lcpObserverInstalled) {
    return;
  }
  ctx.__lcpObserverInstalled = true;
  await page.context().addInitScript(() => {
    // Reset on every new document — the observer below is registered
    // fresh per page, but the array must be empty at navigation start.
    (window as any).__lcpEntries = [] as PerformanceEntry[];
    try {
      const obs = new PerformanceObserver((list) => {
        for (const e of list.getEntries()) {
          (window as any).__lcpEntries.push(e);
        }
      });
      // `buffered: true` ensures we get LCP entries that fired before
      // observer registration (e.g. very fast pages).
      obs.observe({ type: 'largest-contentful-paint', buffered: true });
    } catch {
      // Older browsers without LCP support — fall through, lcp will be 0.
    }
  });
}

/// Wait for LCP to stabilise. Chromium can keep refining the LCP
/// candidate as more elements paint; reading too early gives an
/// artificially low value. Poll `__lcpEntries.length` until it
/// stops growing for `STABLE_MS`, capped at `MAX_WAIT_MS` so the
/// test cannot hang on adversarial pages.
async function waitForLcpStable(page: import('@playwright/test').Page): Promise<void> {
  const POLL_MS = 50;
  const STABLE_MS = 200;
  const MAX_WAIT_MS = 2000;
  const start = Date.now();
  let prevLen = -1;
  let stableFor = 0;
  while (Date.now() - start < MAX_WAIT_MS) {
    await page.waitForTimeout(POLL_MS);
    const curLen = await page.evaluate(
      () => ((window as any).__lcpEntries as unknown[] | undefined)?.length ?? 0,
    );
    if (curLen === prevLen) {
      stableFor += POLL_MS;
      if (stableFor >= STABLE_MS) {
        return;
      }
    } else {
      stableFor = 0;
      prevLen = curLen;
    }
  }
}

async function measure(page: import('@playwright/test').Page, url: string): Promise<PageMetrics> {
  await ensureLcpObserverInstalled(page);
  await page.goto(url, { waitUntil: 'load' });
  await waitForLcpStable(page);

  return page.evaluate(async () => {
    const nav = performance.getEntriesByType('navigation')[0] as PerformanceNavigationTiming | undefined;
    const paints = performance.getEntriesByType('paint') as PerformancePaintTiming[];
    const fcp = paints.find((p) => p.name === 'first-contentful-paint');

    // LCP comes from the PerformanceObserver installed via the
    // context's init script (registered once via
    // `ensureLcpObserverInstalled`).
    const lcpEntries = ((window as any).__lcpEntries || []) as PerformanceEntry[];
    const lcp = lcpEntries.length ? lcpEntries[lcpEntries.length - 1] : undefined;

    return {
      ttfbMs: nav ? nav.responseStart - nav.requestStart : 0,
      fcpMs: fcp ? fcp.startTime : 0,
      lcpMs: lcp ? (lcp as any).renderTime || (lcp as any).loadTime || lcp.startTime : 0,
      dclMs: nav ? nav.domContentLoadedEventEnd - nav.startTime : 0,
      loadMs: nav ? nav.loadEventEnd - nav.startTime : 0,
      bodyLen: nav ? nav.encodedBodySize : 0,
    };
  });
}

function median(xs: number[]): number {
  const sorted = [...xs].sort((a, b) => a - b);
  return sorted[Math.floor(sorted.length / 2)];
}

function fmt(n: number): string {
  return n.toFixed(1).padStart(8) + ' ms';
}

function snapshotPath(name: string): string {
  // tests/ -> ../../../../target/bench-baselines/
  return resolve(
    __dirname,
    '..',
    '..',
    '..',
    '..',
    'target',
    'bench-baselines',
    `browser-${name}.json`,
  );
}

function pctChange(base: number, current: number): number {
  if (base === 0) return 0;
  return ((current - base) / base) * 100;
}

function saveSnapshot(name: string, rows: SnapshotRow[]): void {
  const path = snapshotPath(name);
  mkdirSync(dirname(path), { recursive: true });
  const snap: Snapshot = {
    schema: SNAPSHOT_SCHEMA,
    name,
    timestampUnix: Math.floor(Date.now() / 1000),
    rows,
  };
  writeFileSync(path, JSON.stringify(snap, null, 2));
  console.log(`\n✔ Baseline saved to ${path}`);
}

function loadSnapshot(name: string): Snapshot | null {
  const path = snapshotPath(name);
  if (!existsSync(path)) {
    console.log(`\ncompare: baseline '${name}' not found at ${path} — run with WEBUI_BENCH_SAVE=${name} first`);
    return null;
  }
  const snap = JSON.parse(readFileSync(path, 'utf-8')) as Snapshot;
  if (snap.schema !== SNAPSHOT_SCHEMA) {
    console.log(`\ncompare: baseline '${name}' has schema ${snap.schema} (expected ${SNAPSHOT_SCHEMA}); regenerate`);
    return null;
  }
  return snap;
}

function printDiff(current: SnapshotRow[], baseline: Snapshot): void {
  console.log(`\nDiff vs baseline '${baseline.name}':`);
  console.log(
    'Scenario                                  | Path      |   TTFB Δ% |    FCP Δ% |    LCP Δ% |   load Δ%',
  );
  console.log(
    '------------------------------------------+-----------+-----------+-----------+-----------+-----------',
  );
  for (const cur of current) {
    const base = baseline.rows.find((b) => b.scenario === cur.scenario && b.path === cur.path);
    if (!base) {
      console.log(`${cur.scenario.padEnd(42)}| ${cur.path.padEnd(9)} | (new)`);
      continue;
    }
    const t = pctChange(base.ttfbMsMedian, cur.ttfbMsMedian).toFixed(1).padStart(8);
    const f = pctChange(base.fcpMsMedian, cur.fcpMsMedian).toFixed(1).padStart(8);
    const l = pctChange(base.lcpMsMedian, cur.lcpMsMedian).toFixed(1).padStart(8);
    const ld = pctChange(base.loadMsMedian, cur.loadMsMedian).toFixed(1).padStart(8);
    console.log(
      `${cur.scenario.padEnd(42)}| ${cur.path.padEnd(9)} | ${t}% | ${f}% | ${l}% | ${ld}%`,
    );
  }
  console.log('\nNegative Δ% = improvement; positive = regression. Browser metrics are noisy; treat <±5% as noise.\n');
}

test.describe('Browser-perceived metrics: buffered vs streaming SSR', () => {
  test('captures TTFB / FCP / LCP / DCL / load for all scenarios', async ({ page }) => {
    const results: Record<string, Record<string, PageMetrics[]>> = {};

    for (const { delay, label } of SCENARIOS) {
      results[label] = { buffered: [], streaming: [] };
      for (let i = 0; i < ITERS; i++) {
        results[label].buffered.push(await measure(page, `/buf?delay_us=${delay}`));
        results[label].streaming.push(await measure(page, `/stream?delay_us=${delay}`));
      }
    }

    // Build snapshot rows.
    const snapshotRows: SnapshotRow[] = [];
    for (const { label } of SCENARIOS) {
      for (const path of ['buffered', 'streaming'] as const) {
        const samples = results[label][path];
        snapshotRows.push({
          scenario: label,
          path,
          ttfbMsMedian: median(samples.map((s) => s.ttfbMs)),
          fcpMsMedian: median(samples.map((s) => s.fcpMs)),
          lcpMsMedian: median(samples.map((s) => s.lcpMs)),
          dclMsMedian: median(samples.map((s) => s.dclMs)),
          loadMsMedian: median(samples.map((s) => s.loadMs)),
          bodyBytes: samples[0].bodyLen,
          iters: ITERS,
        });
      }
    }

    // Print results.
    const lines: string[] = [];
    lines.push('');
    lines.push('Browser-perceived metrics (median across ' + ITERS + ' iterations):');
    lines.push('');
    lines.push(
      'Scenario                                  | Path      |     TTFB |      FCP |      LCP |      DCL |     load |    bytes',
    );
    lines.push(
      '------------------------------------------+-----------+----------+----------+----------+----------+----------+---------',
    );
    for (const row of snapshotRows) {
      lines.push(
        `${row.scenario.padEnd(42)}| ${row.path.padEnd(9)} | ${fmt(row.ttfbMsMedian)} | ${fmt(row.fcpMsMedian)} | ${fmt(row.lcpMsMedian)} | ${fmt(row.dclMsMedian)} | ${fmt(row.loadMsMedian)} | ${String(row.bodyBytes).padStart(7)}`,
      );
      if (row.path === 'streaming') {
        lines.push(
          '                                          |           |          |          |          |          |          |         ',
        );
      }
    }
    lines.push('');
    lines.push('Notes:');
    lines.push('  * TTFB = responseStart − requestStart (PerformanceNavigationTiming)');
    lines.push('  * FCP / LCP from PerformanceObserver inside Chromium');
    lines.push('  * DCL / load from PerformanceNavigationTiming');
    lines.push('  * Identical HTML on both endpoints (verified below)');
    lines.push('');
    console.log(lines.join('\n'));

    // Sanity check: both endpoints must serve byte-identical HTML.
    const bufBody = await page.evaluate(async () => {
      const r = await fetch('/buf?delay_us=0');
      return await r.text();
    });
    const streamBody = await page.evaluate(async () => {
      const r = await fetch('/stream?delay_us=0');
      return await r.text();
    });
    expect(streamBody).toBe(bufBody);

    // Hard regression check: at the 100 ms render scenario streaming
    // TTFB must be at least 5x lower than buffered.
    const slow = snapshotRows.filter((r) => r.scenario === '200µs/write (~100 ms render)');
    const buf = slow.find((r) => r.path === 'buffered')!;
    const stream = slow.find((r) => r.path === 'streaming')!;
    expect(stream.ttfbMsMedian).toBeLessThan(buf.ttfbMsMedian / 5);

    // Baseline save / compare via env vars.
    const saveName = process.env.WEBUI_BENCH_SAVE;
    const compareName = process.env.WEBUI_BENCH_COMPARE;
    if (saveName) {
      saveSnapshot(saveName, snapshotRows);
    }
    if (compareName) {
      const baseline = loadSnapshot(compareName);
      if (baseline) {
        printDiff(snapshotRows, baseline);
      }
    }
  });
});

