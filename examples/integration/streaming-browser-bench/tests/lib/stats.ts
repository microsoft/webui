// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Statistics + baseline snapshot I/O for the progressive-hydration matrix.
 *
 * This uses a distinct snapshot filename/schema from the transport bench
 * (`browser-hydration-<name>.json` vs `browser-<name>.json`) so saving a
 * hydration baseline never clobbers a transport snapshot and vice versa.
 */

import { mkdirSync, readFileSync, writeFileSync, existsSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));

/** Bump when the row/bundle shape changes so stale baselines are rejected. */
export const HYDRATION_SNAPSHOT_SCHEMA = 1;

export interface BundleSizes {
  ordinaryMinifiedBytes: number;
  ordinaryGzipBytes: number;
  streamingMinifiedBytes: number;
  streamingGzipBytes: number;
  streamingIncrementalBytes: number;
  streamingIncrementalGzipBytes: number;
}

export interface HydrationRow {
  scenario: string;
  boundaryCount: number;
  runs: number;
  cpuMsMedian: number;
  cpuMsP95: number;
  elapsedMsMedian: number;
  elapsedMsP95: number;
  peakHeapDeltaBytesMedian: number | null;
  retainedHeapDeltaBytesMedian: number | null;
  maxLongTaskMsMedian: number | null;
}

export interface HydrationSnapshot {
  schema: number;
  name: string;
  timestampUnix: number;
  bundle: BundleSizes;
  rows: HydrationRow[];
}

export function median(xs: number[]): number {
  if (xs.length === 0) return 0;
  const sorted = [...xs].sort((a, b) => a - b);
  return sorted[Math.floor(sorted.length / 2)];
}

/** Nearest-rank p95 (index `ceil(0.95 * n) - 1`). */
export function p95(xs: number[]): number {
  if (xs.length === 0) return 0;
  const sorted = [...xs].sort((a, b) => a - b);
  const rank = Math.ceil(0.95 * sorted.length) - 1;
  return sorted[Math.min(rank, sorted.length - 1)];
}

/** Median over samples that are non-null, or null when every sample is null. */
export function medianNullable(xs: Array<number | null>): number | null {
  const present = xs.filter((x): x is number => x !== null);
  return present.length ? median(present) : null;
}

function snapshotPath(name: string): string {
  // tests/lib/ -> ../../../../../target/bench-baselines/
  return resolve(
    here,
    '..',
    '..',
    '..',
    '..',
    '..',
    'target',
    'bench-baselines',
    `browser-hydration-${name}.json`,
  );
}

export function saveSnapshot(name: string, bundle: BundleSizes, rows: HydrationRow[]): void {
  const path = snapshotPath(name);
  mkdirSync(dirname(path), { recursive: true });
  const snapshot: HydrationSnapshot = {
    schema: HYDRATION_SNAPSHOT_SCHEMA,
    name,
    timestampUnix: Math.floor(Date.now() / 1000),
    bundle,
    rows,
  };
  writeFileSync(path, JSON.stringify(snapshot, null, 2));
  console.log(`\n[hydration] baseline saved to ${path}`);
}

export function loadSnapshot(name: string, required = false): HydrationSnapshot | null {
  const path = snapshotPath(name);
  if (!existsSync(path)) {
    return unavailableSnapshot(
      `baseline '${name}' not found at ${path} (run WEBUI_BENCH_SAVE=${name} first)`,
      required,
    );
  }
  const snapshot = JSON.parse(readFileSync(path, 'utf-8')) as HydrationSnapshot;
  if (snapshot.schema !== HYDRATION_SNAPSHOT_SCHEMA) {
    return unavailableSnapshot(
      `baseline '${name}' schema ${snapshot.schema} != ${HYDRATION_SNAPSHOT_SCHEMA}; regenerate it`,
      required,
    );
  }
  return snapshot;
}

function unavailableSnapshot(message: string, required: boolean): null {
  if (required) {
    throw new Error(`[hydration] compare: ${message}`);
  }
  console.log(`\n[hydration] compare: ${message}`);
  return null;
}

function pctChange(base: number, current: number): number {
  if (base === 0) return 0;
  return ((current - base) / base) * 100;
}

/** Tolerances for baseline comparison. CPU/elapsed use a relative percentage;
 *  memory uses an absolute floor OR a relative percentage (whichever is larger)
 *  so growth from a zero/small baseline is still detected and division-by-zero
 *  can never mask a regression. */
export interface CompareTolerances {
  /** Relative tolerance (%) for CPU and elapsed medians. */
  relativePct: number;
  /** Retained-heap allowance: `max(retainedAbsBytes, |base| * retainedPct%)`. */
  retainedAbsBytes: number;
  retainedPct: number;
  /** Peak-heap allowance: `max(peakAbsBytes, |base| * peakPct%)`. */
  peakAbsBytes: number;
  peakPct: number;
}

function fmtPct(base: number, current: number): string {
  return `${pctChange(base, current).toFixed(1).padStart(7)}%`;
}

/** Signed KiB delta, or "     n/a" when either sample is null. */
function fmtDeltaKiB(base: number | null, current: number | null): string {
  if (base === null || current === null) return '     n/a';
  const deltaKiB = (current - base) / 1024;
  const sign = deltaKiB >= 0 ? '+' : '';
  return `${sign}${deltaKiB.toFixed(1)}KiB`.padStart(11);
}

/**
 * Compare a nullable memory metric against a baseline. Returns a regression
 * string when `current - base` exceeds `max(absFloor, |base| * pct%)`, or null
 * when within tolerance / not comparable. Skips (returns null) only when either
 * sample is null. The absolute floor is always applied, so growth from a
 * zero/small baseline is detected rather than masked by a percentage of zero.
 */
function memoryRegression(
  label: string,
  metric: string,
  base: number | null,
  current: number | null,
  absFloor: number,
  pct: number,
): string | null {
  if (base === null || current === null) return null;
  const allowance = Math.max(absFloor, Math.abs(base) * (pct / 100));
  const delta = current - base;
  if (delta > allowance) {
    return `${label}: ${metric} +${(delta / 1024).toFixed(1)}KiB > ${(allowance / 1024).toFixed(1)}KiB`;
  }
  return null;
}

/**
 * Print a delta table against a baseline (CPU, elapsed, retained heap, peak
 * heap) and return any regressions. CPU/elapsed use the relative tolerance;
 * retained/peak use absolute-floored allowances so a *uniform* memory
 * regression fails even when the within-run N=1/10/100 slope stays flat.
 * Callers only treat the returned regressions as fatal under the strict flag.
 */
export function diffAgainst(
  current: HydrationRow[],
  baseline: HydrationSnapshot,
  tol: CompareTolerances,
): string[] {
  const regressions: string[] = [];
  console.log(
    `\n[hydration] diff vs baseline '${baseline.name}'`
    + ` (cpu/elapsed +/-${tol.relativePct}%, retained max(${(tol.retainedAbsBytes / 1024).toFixed(0)}KiB, ${tol.retainedPct}%),`
    + ` peak max(${(tol.peakAbsBytes / 1024).toFixed(0)}KiB, ${tol.peakPct}%)):`,
  );
  console.log('scenario                          |   cpu d% | elapsed d% |  retained d |     peak d');
  console.log('----------------------------------+----------+------------+-------------+-----------');
  for (const row of current) {
    const base = baseline.rows.find((b) => b.scenario === row.scenario);
    if (!base) {
      console.log(`${row.scenario.padEnd(34)}| (new)`);
      continue;
    }
    console.log(
      `${row.scenario.padEnd(34)}| ${fmtPct(base.cpuMsMedian, row.cpuMsMedian)} | ${fmtPct(base.elapsedMsMedian, row.elapsedMsMedian)}  |`
      + ` ${fmtDeltaKiB(base.retainedHeapDeltaBytesMedian, row.retainedHeapDeltaBytesMedian)} | ${fmtDeltaKiB(base.peakHeapDeltaBytesMedian, row.peakHeapDeltaBytesMedian)}`,
    );

    const cpuDelta = pctChange(base.cpuMsMedian, row.cpuMsMedian);
    if (cpuDelta > tol.relativePct) {
      regressions.push(`${row.scenario}: cpu +${cpuDelta.toFixed(1)}% > ${tol.relativePct}%`);
    }
    const elapsedDelta = pctChange(base.elapsedMsMedian, row.elapsedMsMedian);
    if (elapsedDelta > tol.relativePct) {
      regressions.push(`${row.scenario}: elapsed +${elapsedDelta.toFixed(1)}% > ${tol.relativePct}%`);
    }
    const retainedReg = memoryRegression(
      row.scenario, 'retained',
      base.retainedHeapDeltaBytesMedian, row.retainedHeapDeltaBytesMedian,
      tol.retainedAbsBytes, tol.retainedPct,
    );
    if (retainedReg) regressions.push(retainedReg);
    const peakReg = memoryRegression(
      row.scenario, 'peak',
      base.peakHeapDeltaBytesMedian, row.peakHeapDeltaBytesMedian,
      tol.peakAbsBytes, tol.peakPct,
    );
    if (peakReg) regressions.push(peakReg);
  }
  return regressions;
}
