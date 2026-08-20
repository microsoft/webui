// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Progressive streaming-hydration performance / memory matrix.
 *
 * Unlike `browser_metrics.spec.ts` (which measures transport: buffered `/buf`
 * vs streamed `/stream`), this spec measures the *real* streaming
 * coordinator (`packages/webui-framework/src/streaming.ts`) driving the *real*
 * `WebUIElement` hydration path — not a synthetic clone. It bundles the actual
 * framework sources in-memory with esbuild and runs an equal-total-work matrix:
 * the same 1,500 real `bench-island` SSR roots and the same total streamed state
 * bytes are delivered across 1, 3, 10, and 100 boundaries, plus an ordinary
 * one-shot control. Only the boundary count changes, so component-hydration CPU
 * and retained heap should stay flat while the coordinator absorbs the extra
 * boundary bookkeeping.
 *
 * Deterministic correctness is always enforced (equal live roots, zero residual
 * scaffolding, no globally-published streamed state, coordinator-free ordinary
 * bundle, bounded coordinator bundle bytes). The noisy performance/memory gates
 * (component CPU <= single-boundary streaming one-shot * 1.05, bounded retained-
 * heap slope, bounded peak heap, linear elapsed growth) are opt-in via
 * `WEBUI_STREAMING_HYDRATION_ENFORCE=1` so ordinary CI stays stable.
 *
 * # Baseline workflow (distinct from the transport snapshot)
 *
 *   WEBUI_BENCH_SAVE=before    pnpm test:hydration
 *   WEBUI_BENCH_COMPARE=before pnpm test:hydration
 *   WEBUI_STREAMING_HYDRATION_ENFORCE=1 WEBUI_BENCH_COMPARE=before pnpm test:hydration
 *
 * Hydration baselines live at `target/bench-baselines/browser-hydration-<name>.json`.
 */

import { test, expect, type Browser, type CDPSession, type Page } from '@playwright/test';

import {
  buildOrdinaryScenario,
  buildStateDeliveryScenario,
  buildStateUpdateScenario,
  buildStreamingScenario,
  ordinaryBaseHtml,
  streamingBaseHtml,
  BOUNDARY_COUNTS,
  TOTAL_ROOTS,
  TOTAL_STATE_VALUE_BYTES,
  type StreamingScenario,
} from './lib/scenarios.js';
import { buildFixtures, coordinatorTokensIn, type BuiltFixtures } from './lib/fixtures.js';
import { runScenario, type DriveConfig, type RunMetrics } from './lib/drivers.js';
import {
  diffAgainst,
  loadSnapshot,
  median,
  medianNullable,
  p95,
  saveSnapshot,
  type BundleSizes,
  type CompareTolerances,
  type HydrationRow,
} from './lib/stats.js';

const ENFORCE = process.env.WEBUI_STREAMING_HYDRATION_ENFORCE === '1';

/**
 * Repeated runs per primary scenario for median / p95.
 *
 * Nearest-rank p95 needs enough samples to be a meaningful tail statistic (with
 * 5 samples p95 == max). Strict/enforced runs therefore use at least 20 samples;
 * the count is overridable via `WEBUI_STREAMING_HYDRATION_RUNS` but is floored at
 * 20 under enforce. Normal (non-enforced) runs default to a cheap 5.
 */
const ENFORCE_MIN_RUNS = 20;
const ENV_RUNS = Number.parseInt(process.env.WEBUI_STREAMING_HYDRATION_RUNS ?? '', 10);
const REQUESTED_RUNS = Number.isFinite(ENV_RUNS) && ENV_RUNS > 0 ? ENV_RUNS : undefined;
const RUNS = ENFORCE
  ? Math.max(ENFORCE_MIN_RUNS, REQUESTED_RUNS ?? ENFORCE_MIN_RUNS)
  : (REQUESTED_RUNS ?? 5);

/** Component-hydration CPU tolerance vs the single-boundary streaming one-shot. */
const CPU_TOLERANCE_PCT = 5;
/** Small absolute timing noise floor added to the CPU gate (ms) — kept well
 *  below the observed multi-millisecond control CPU so a real >5% regression
 *  fails rather than being absorbed by the floor. */
const CPU_NOISE_FLOOR_MS = 0.25;
/** Retained-heap slope allowance across boundary counts (strict gate). */
const RETAINED_SLOPE_ABS_BYTES = 64 * 1024;
const RETAINED_SLOPE_PCT = 2;
/** Peak-heap strict gate: a higher boundary count must not materially raise the
 *  peak working set. The single-boundary arm (all roots delivered at once) is the
 *  largest-boundary reference; every arm may exceed it by at most
 *  `max(512KiB, 15%)`. This is referenced against streaming itself, not the
 *  one-shot control, because the coordinator legitimately carries transient
 *  scaffolding the control never allocates. */
const PEAK_HEAP_ABS_FLOOR_BYTES = 512 * 1024;
const PEAK_HEAP_TOLERANCE_PCT = 15;
/** Deterministic production coordinator size caps. Raising either requires
 *  explicit review because every streaming application pays these bytes.
 *  The v2 coordinator includes checkpoint declaration/span validation, retained
 *  roots for state updates, span completion, terminal cleanup, commit marks, and
 *  the opt-in time-sliced drain. The reviewed production bundle is 17,147 bytes
 *  minified / 5,885 bytes gzip incrementally; these caps leave roughly 4%
 *  headroom, so further growth still fails. */
const STREAMING_INCREMENTAL_MINIFIED_CAP_BYTES = 17.5 * 1024;
const STREAMING_INCREMENTAL_GZIP_CAP_BYTES = 6 * 1024;
/** Marginal elapsed-time cap per added boundary. The relative allowance scales
 *  with slower hosts while the absolute floor absorbs sub-millisecond noise. */
const COORDINATOR_MARGINAL_ABS_CAP_MS = 0.25;
const COORDINATOR_MARGINAL_RELATIVE_PCT = 1;
/** Baseline-compare CPU/elapsed relative regression tolerance (strict gate). */
const COMPARE_TOLERANCE_PCT = 10;
/** Baseline-compare memory allowances. Absolute floors catch a uniform memory
 *  regression that leaves the within-run N=1/10/100 slope flat, and are always
 *  applied so growth from a zero/small baseline is never masked by a percentage
 *  of zero. */
const COMPARE_RETAINED_ABS_BYTES = 64 * 1024;
const COMPARE_RETAINED_PCT = 10;
const COMPARE_PEAK_ABS_BYTES = 512 * 1024;
const COMPARE_PEAK_PCT = 15;

interface AggregatedRow extends HydrationRow {
  /** Per-round component CPU, aligned across arms for paired comparisons. */
  cpuSamples: number[];
  /** Per-round elapsed time, aligned across arms for paired comparisons. */
  elapsedSamples: number[];
  /** Per-run retained-heap deltas (bytes) after forced GC. */
  retainedSamples: Array<number | null>;
  /** Per-run peak-heap deltas (bytes) sampled while boundaries commit. */
  peakSamples: Array<number | null>;
}

interface RunResult {
  metrics: RunMetrics;
  retainedHeapDeltaBytes: number | null;
  /** Browser console errors + uncaught page errors captured during the run.
   *  The coordinator only logs on real failures, so this must stay empty. */
  errors: string[];
}

interface PrimaryArm {
  readonly kind: 'streaming' | 'ordinary';
  readonly label: string;
  readonly boundaryCount: number;
  readonly baseHtml: string;
  readonly bundleCode: string;
  readonly config: DriveConfig;
  readonly cpu: number[];
  readonly elapsed: number[];
  readonly peak: Array<number | null>;
  readonly retained: Array<number | null>;
  readonly longTask: Array<number | null>;
}

async function getHeapUsage(session: CDPSession): Promise<number | null> {
  try {
    await session.send('HeapProfiler.collectGarbage');
    const usage = (await session.send('Runtime.getHeapUsage')) as { usedSize: number };
    return usage.usedSize;
  } catch {
    // Forced GC / heap usage is Chromium-only; tolerate its absence explicitly.
    return null;
  }
}

/** Run one scenario once in a fresh page (fresh JS realm => fresh coordinator
 *  and lifecycle state) and measure forced-GC retained heap around the drive. */
async function runOnce(
  browser: Browser,
  baseHtml: string,
  bundleCode: string,
  config: DriveConfig,
): Promise<RunResult> {
  const page: Page = await browser.newPage();
  const errors: string[] = [];
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(`console.error: ${message.text()}`);
  });
  page.on('pageerror', (error) => errors.push(`pageerror: ${error.message}`));
  try {
    await page.setContent(baseHtml, { waitUntil: 'load' });
    await page.addScriptTag({ content: bundleCode });

    const session = await page.context().newCDPSession(page);
    const before = await getHeapUsage(session);
    const metrics = await page.evaluate(runScenario, config);
    const after = await getHeapUsage(session);
    await session.detach().catch(() => {
      /* session already gone with the page — nothing to bound */
    });

    const retainedHeapDeltaBytes =
      before !== null && after !== null ? after - before : null;
    return { metrics, retainedHeapDeltaBytes, errors };
  } finally {
    await page.close();
  }
}

/** Deterministic correctness invariants that must hold on every run. */
function assertDeterministic(
  kind: 'streaming' | 'ordinary',
  metrics: RunMetrics,
  expectedRoots = TOTAL_ROOTS,
): void {
  expect(metrics.liveRootCount, 'equal live root count').toBe(expectedRoots);
  // Prove every root genuinely hydrated, not merely survived scaffold removal:
  // the instrumented subclass counted a successful super hydration exactly once
  // per instance, and the post-measurement reactive probe confirmed each bound
  // <span> re-rendered from setState. A partial/no-op activation fails here even
  // though live-root-count and aggregate cpu would still pass.
  expect(metrics.hydratedCount, 'every root reported a successful hydration').toBe(expectedRoots);
  expect(metrics.verifiedReactiveCount, 'every root proven reactive by setState probe').toBe(expectedRoots);
  // Guard against silently measuring nothing: if activation ever failed while
  // scaffolding still cleared, cpu would stay 0. Real hydration of every root
  // always accumulates measurable component CPU.
  expect(metrics.cpuMs, 'real component hydration work measured').toBeGreaterThan(0);
  expect(metrics.scaffoldScripts, 'zero residual boundary scripts').toBe(0);
  expect(metrics.scaffoldSentinels, 'zero residual sentinels').toBe(0);
  expect(metrics.scaffoldComments, 'zero residual boundary comments').toBe(0);
  expect(metrics.scaffoldDataWs, 'zero residual data-ws markers').toBe(0);
  if (kind === 'streaming') {
    expect(metrics.streamedStatePopulated, 'boundary state never published globally').toBe(false);
  }
}

function assertEnforcedHeapSamples(
  result: RunResult,
  scenario: string,
  run: number,
): void {
  if (!ENFORCE) return;
  expect(
    result.metrics.peakHeapDeltaBytes,
    `${scenario} run ${run}: enforced peak-heap sample must be available`,
  ).not.toBeNull();
  expect(
    result.retainedHeapDeltaBytes,
    `${scenario} run ${run}: enforced retained-heap sample must be available`,
  ).not.toBeNull();
}

function fmtBytes(n: number | null): string {
  if (n === null) return '     n/a';
  return `${(n / 1024).toFixed(1)}KiB`.padStart(9);
}

function fmtMs(n: number | null): string {
  return n === null ? '    n/a' : n.toFixed(2).padStart(7);
}

/** Rotate arm order every round, reversing each complete cycle. With the
 * default five arms and five runs, every arm occupies every position once. */
function balancedArmOrder(armCount: number, round: number): number[] {
  const order = Array.from({ length: armCount }, (_, index) => index);
  if (Math.floor(round / armCount) % 2 === 1) order.reverse();
  const shift = round % armCount;
  return order.slice(shift).concat(order.slice(0, shift));
}

function parseBoundaryRecord(fragment: string): unknown[] {
  const prefix = '<script type="application/json" data-webui-boundary>';
  const start = fragment.indexOf(prefix);
  const end = fragment.indexOf('</script>', start + prefix.length);
  if (start < 0 || end < 0) {
    throw new Error('benchmark fragment is missing its boundary record');
  }
  const parsed: unknown = JSON.parse(fragment.slice(start + prefix.length, end));
  if (!Array.isArray(parsed)) {
    throw new Error('benchmark boundary record is not an array');
  }
  return parsed;
}

test.describe('progressive streaming hydration matrix', () => {
  test('generates gapless v2 checkpoint, update, and terminal records', () => {
    const checkpointScenario = buildStreamingScenario(3, 'flat', 'eager', true);
    const checkpointRecords = [
      ...checkpointScenario.boundaries,
      checkpointScenario.terminal,
    ].map(parseBoundaryRecord);

    expect(checkpointRecords.map((record) => record.length)).toEqual([5, 5, 5, 5, 5]);
    expect(checkpointRecords.map((record) => record[0])).toEqual([2, 2, 2, 2, 2]);
    expect(checkpointRecords.map((record) => record[1])).toEqual([0, 1, 2, 3, 4]);
    expect(checkpointRecords.map((record) => record[2])).toEqual([0, 0, 0, 0, 4]);
    expect(checkpointRecords.map((record) => record[3])).toEqual([0, 1, 2, 3, 0]);
    for (const record of checkpointRecords.slice(0, -1)) {
      const bootstrap = record[4] as Record<string, unknown>;
      expect(bootstrap.declarationId).toBe(0);
      expect(
        Object.prototype.hasOwnProperty.call(bootstrap, 'enclosingSpanInstanceId'),
        'flat entry checkpoints omit enclosingSpanInstanceId',
      ).toBe(false);
    }
    expect(checkpointRecords[4]).toEqual([2, 4, 4, 0, {}]);

    const updateScenario = buildStateUpdateScenario(3, 100);
    const updateRecords = [
      ...updateScenario.boundaries,
      updateScenario.terminal,
    ].map(parseBoundaryRecord);
    expect(updateRecords.map((record) => record[0])).toEqual([2, 2, 2, 2, 2]);
    expect(updateRecords.map((record) => record[1])).toEqual([0, 1, 2, 3, 4]);
    expect(updateRecords.map((record) => record[2])).toEqual([1, 2, 2, 2, 4]);
    expect(updateRecords.map((record) => record[3])).toEqual([0, 0, 0, 0, 0]);
    expect((updateRecords[0][4] as Record<string, unknown>).declarationId).toBe(0);
    expect(updateRecords[4]).toEqual([2, 4, 4, 0, {}]);
  });

  test('measures real coordinator + WebUIElement hydration across boundary counts', async ({ browser }) => {
    const compareName = process.env.WEBUI_BENCH_COMPARE;
    const baseline = compareName ? loadSnapshot(compareName, ENFORCE) : null;
    if (baseline && ENFORCE) {
      const missingMemory = baseline.rows
        .filter(
          (row) =>
            row.peakHeapDeltaBytesMedian === null
            || row.retainedHeapDeltaBytesMedian === null,
        )
        .map((row) => row.scenario);
      expect(
        missingMemory,
        'an enforced baseline comparison requires peak and retained heap for every row',
      ).toEqual([]);
    }

    // ── 1. Bundle the real framework sources ─────────────────────────
    const fixtures: BuiltFixtures = await buildFixtures();

    const ordinaryTokens = coordinatorTokensIn(fixtures.ordinary.code);
    expect(ordinaryTokens, 'ordinary bundle must be coordinator-free').toEqual([]);
    // Sanity: the streaming bundle *does* carry the coordinator it advertises.
    expect(coordinatorTokensIn(fixtures.streaming.code).length).toBeGreaterThan(0);
    expect(
      fixtures.streamingIncrementalBytes,
      'streaming coordinator incremental minified bytes stay within the reviewed 17.5KiB cap',
    ).toBeLessThanOrEqual(STREAMING_INCREMENTAL_MINIFIED_CAP_BYTES);
    expect(
      fixtures.streamingIncrementalGzipBytes,
      'streaming coordinator incremental gzip bytes stay within the reviewed 6KiB cap',
    ).toBeLessThanOrEqual(STREAMING_INCREMENTAL_GZIP_CAP_BYTES);

    const bundle: BundleSizes = {
      ordinaryMinifiedBytes: fixtures.ordinary.minifiedBytes,
      ordinaryGzipBytes: fixtures.ordinary.gzipBytes,
      streamingMinifiedBytes: fixtures.streaming.minifiedBytes,
      streamingGzipBytes: fixtures.streaming.gzipBytes,
      streamingIncrementalBytes: fixtures.streamingIncrementalBytes,
      streamingIncrementalGzipBytes: fixtures.streamingIncrementalGzipBytes,
    };

    console.log('\nBundle sizes (real framework sources, minified, __WEBUI_DEV__=false):');
    console.log('fixture     |  minified |      gzip');
    console.log('------------+-----------+----------');
    console.log(`ordinary    | ${String(bundle.ordinaryMinifiedBytes).padStart(9)} | ${String(bundle.ordinaryGzipBytes).padStart(8)}`);
    console.log(`streaming   | ${String(bundle.streamingMinifiedBytes).padStart(9)} | ${String(bundle.streamingGzipBytes).padStart(8)}`);
    console.log(`incremental | ${String(bundle.streamingIncrementalBytes).padStart(9)} | ${String(bundle.streamingIncrementalGzipBytes).padStart(8)}`);

    // ── 2. Verify equal-total-work invariants across scenarios ───────
    const control = buildOrdinaryScenario();
    const primary: StreamingScenario[] = BOUNDARY_COUNTS.map((count) =>
      buildStreamingScenario(count, 'flat', 'eager', true),
    );
    for (const scenario of primary) {
      expect(scenario.totalRoots, 'equal total roots').toBe(TOTAL_ROOTS);
      expect(scenario.totalStateChars, 'equal projected state value bytes').toBe(TOTAL_STATE_VALUE_BYTES);
    }
    expect(control.totalRoots).toBe(TOTAL_ROOTS);
    expect(control.totalStateChars).toBe(TOTAL_STATE_VALUE_BYTES);

    // ── 3. Run warmed, interleaved, paired primary rounds ────────────
    const arms: PrimaryArm[] = [
      {
        kind: 'ordinary',
        label: control.label,
        boundaryCount: 0,
        baseHtml: ordinaryBaseHtml(control.bootstrapHtml),
        bundleCode: fixtures.ordinary.code,
        config: { mode: 'ordinary', bodyHtml: control.rootsHtml },
        cpu: [],
        elapsed: [],
        peak: [],
        retained: [],
        longTask: [],
      },
      ...primary.map((scenario): PrimaryArm => ({
        kind: 'streaming',
        label: scenario.label,
        boundaryCount: scenario.boundaryCount,
        baseHtml: streamingBaseHtml(),
        bundleCode: fixtures.streaming.code,
        config: {
          mode: 'streaming',
          boundaries: scenario.boundaries,
          terminal: scenario.terminal,
          timing: scenario.timing,
        },
        cpu: [],
        elapsed: [],
        peak: [],
        retained: [],
        longTask: [],
      })),
    ];

    // Warm every production path once. These samples are deliberately discarded
    // so first-use JIT and browser process startup do not bias the first arm.
    for (const armIndex of balancedArmOrder(arms.length, 0)) {
      const arm = arms[armIndex];
      const result = await runOnce(browser, arm.baseHtml, arm.bundleCode, arm.config);
      assertDeterministic(arm.kind, result.metrics);
      expect(result.errors, `no browser console/page errors (${arm.label} warmup)`).toEqual([]);
    }

    // Every round contains every arm. Rotation removes fixed position bias, and
    // aligned sample indexes make strict comparisons paired within one round.
    for (let round = 0; round < RUNS; round++) {
      for (const armIndex of balancedArmOrder(arms.length, round)) {
        const arm = arms[armIndex];
        const result = await runOnce(browser, arm.baseHtml, arm.bundleCode, arm.config);
        assertEnforcedHeapSamples(result, arm.label, round);
        assertDeterministic(arm.kind, result.metrics);
        expect(result.errors, `no browser console/page errors (${arm.label})`).toEqual([]);
        arm.cpu.push(result.metrics.cpuMs);
        arm.elapsed.push(result.metrics.elapsedMs);
        arm.peak.push(result.metrics.peakHeapDeltaBytes);
        arm.retained.push(result.retainedHeapDeltaBytes);
        arm.longTask.push(result.metrics.maxLongTaskMs);
      }
    }

    const rows: AggregatedRow[] = arms.map((arm) =>
      aggregate(
        arm.label,
        arm.boundaryCount,
        RUNS,
        arm.cpu,
        arm.elapsed,
        arm.peak,
        arm.retained,
        arm.longTask,
      ),
    );

    // ── 4. Print the matrix ──────────────────────────────────────────
    console.log(`\nProgressive hydration matrix (${TOTAL_ROOTS} roots, ${(TOTAL_STATE_VALUE_BYTES / 1024).toFixed(0)}KiB projected state, median of ${RUNS}):`);
    console.log('scenario                       | bnd | cpu md | cpu p95 | ela md | ela p95 |  peakHeap | retained | longtask');
    console.log('-------------------------------+-----+--------+---------+--------+---------+-----------+----------+---------');
    for (const row of rows) {
      console.log(
        `${row.scenario.padEnd(31)}| ${String(row.boundaryCount).padStart(3)} | ${fmtMs(row.cpuMsMedian)} | ${fmtMs(row.cpuMsP95)} | ${fmtMs(row.elapsedMsMedian)} | ${fmtMs(row.elapsedMsP95)} | ${fmtBytes(row.peakHeapDeltaBytesMedian)} | ${fmtBytes(row.retainedHeapDeltaBytesMedian)} | ${fmtMs(row.maxLongTaskMsMedian)}`,
      );
    }

    // ── 5. Coverage: flat/nested marker ranges + boundary-before-def race
    const coverage: Array<{ label: string; scenario: StreamingScenario }> = [
      { label: 'nested eager (B=10)', scenario: buildStreamingScenario(10, 'nested', 'eager') },
      { label: 'flat race (B=10)', scenario: buildStreamingScenario(10, 'flat', 'race') },
      { label: 'nested race (B=100)', scenario: buildStreamingScenario(100, 'nested', 'race') },
    ];
    console.log('\nCoverage cases (deterministic correctness only):');
    for (const { label, scenario } of coverage) {
      const result = await runOnce(browser, streamingBaseHtml(), fixtures.streaming.code, {
        mode: 'streaming',
        boundaries: scenario.boundaries,
        terminal: scenario.terminal,
        timing: scenario.timing,
      });
      assertDeterministic('streaming', result.metrics);
      expect(result.errors, `no browser console/page errors (${label})`).toEqual([]);
      console.log(
        `  ${label.padEnd(22)} roots=${result.metrics.liveRootCount} hydrated=${result.metrics.hydratedCount} reactive=${result.metrics.verifiedReactiveCount} scaffold=0 globalState=${result.metrics.streamedStatePopulated} longtask=${fmtMs(result.metrics.maxLongTaskMs)}`,
      );
    }

    // Exact state-delivery correctness uses the real WebUIElement activation
    // hook but is kept outside all timed/heap aggregates. Distinct boundary
    // labels make dropped, stale, or cumulative state immediately visible.
    const stateLabels = ['state-alpha', 'state-bravo', 'state-charlie'];
    const rootsPerStateBoundary = 50;
    const stateScenario = buildStateDeliveryScenario(
      stateLabels,
      rootsPerStateBoundary,
    );
    const stateResult = await runOnce(
      browser,
      streamingBaseHtml(),
      fixtures.streaming.code,
      {
        mode: 'streaming',
        boundaries: stateScenario.boundaries,
        terminal: stateScenario.terminal,
        timing: 'eager',
        captureBoundaryState: true,
      },
    );
    assertDeterministic('streaming', stateResult.metrics, stateScenario.totalRoots);
    expect(stateResult.errors, 'no browser console/page errors (state delivery)').toEqual([]);
    expect(stateResult.metrics.receivedBoundaryLabels).toEqual(
      stateLabels.flatMap((label) => Array<string>(rootsPerStateBoundary).fill(label)),
    );

    // ── 6. Server-state update throughput and retention ──────────────
    const updateScenario = buildStateUpdateScenario(100, 100);
    const updateElapsed: number[] = [];
    const updatePeak: Array<number | null> = [];
    const updateRetained: Array<number | null> = [];
    const updateLongTask: Array<number | null> = [];
    for (let run = 0; run < RUNS; run++) {
      const result = await runOnce(
        browser,
        streamingBaseHtml(),
        fixtures.streaming.code,
        {
          mode: 'streaming',
          boundaries: updateScenario.boundaries,
          terminal: updateScenario.terminal,
          timing: 'eager',
          expectedFinalLabel: updateScenario.finalLabel,
        },
      );
      assertEnforcedHeapSamples(result, updateScenario.label, run);
      assertDeterministic('streaming', result.metrics, updateScenario.totalRoots);
      expect(
        result.metrics.finalStateMatchCount,
        'every retained root receives the final server-state record',
      ).toBe(updateScenario.totalRoots);
      expect(
        result.errors,
        `no browser console/page errors (${updateScenario.label})`,
      ).toEqual([]);
      updateElapsed.push(result.metrics.elapsedMs);
      updatePeak.push(result.metrics.peakHeapDeltaBytes);
      updateRetained.push(result.retainedHeapDeltaBytes);
      updateLongTask.push(result.metrics.maxLongTaskMs);
    }
    const updateElapsedMedian = median(updateElapsed);
    console.log(
      `\nServer-state updates (${updateScenario.totalRoots} roots x ${updateScenario.updateCount} updates, median of ${RUNS}):`,
    );
    console.log(
      `  elapsed=${fmtMs(updateElapsedMedian)}ms`
      + ` records/ms=${(updateScenario.updateCount / updateElapsedMedian).toFixed(2)}`
      + ` peak=${fmtBytes(medianNullable(updatePeak))}`
      + ` retained=${fmtBytes(medianNullable(updateRetained))}`
      + ` longtask=${fmtMs(medianNullable(updateLongTask))}`,
    );

    // ── 7. Deterministic streaming-jank guarantee ────────────────────
    // Spreading work across many boundaries/microtasks must avoid a single
    // >50ms long task. Long-task entries only exist for tasks over 50ms, so a
    // null reading means "no long task at all". This is the concrete streaming
    // win over the one-shot control and should hold whenever the observer is
    // supported.
    const highBoundary = rows.find((r) => r.boundaryCount === 100);
    if (highBoundary && highBoundary.maxLongTaskMsMedian !== null) {
      expect(
        highBoundary.maxLongTaskMsMedian,
        '100-boundary streaming must not stall the main thread (>50ms long task)',
      ).toBeLessThanOrEqual(50);
    }

    // ── 8. Strict, opt-in performance / memory gates ─────────────────
    if (ENFORCE) {
      enforceStrictGates(rows);
    } else {
      console.log('\n[hydration] strict gates skipped (set WEBUI_STREAMING_HYDRATION_ENFORCE=1 to enforce).');
    }

    // ── 9. Baseline save / compare ───────────────────────────────────
    const plainRows: HydrationRow[] = rows.map(({
      cpuSamples,
      elapsedSamples,
      retainedSamples,
      peakSamples,
      ...rest
    }) => {
      void cpuSamples;
      void elapsedSamples;
      void retainedSamples;
      void peakSamples;
      return rest;
    });
    const saveName = process.env.WEBUI_BENCH_SAVE;
    if (saveName) {
      saveSnapshot(saveName, bundle, plainRows);
    }
    if (compareName && baseline) {
      const compareTolerances: CompareTolerances = {
        relativePct: COMPARE_TOLERANCE_PCT,
        retainedAbsBytes: COMPARE_RETAINED_ABS_BYTES,
        retainedPct: COMPARE_RETAINED_PCT,
        peakAbsBytes: COMPARE_PEAK_ABS_BYTES,
        peakPct: COMPARE_PEAK_PCT,
      };
      const regressions = diffAgainst(plainRows, baseline, compareTolerances);
      if (regressions.length > 0) {
        console.log(`\n[hydration] ${regressions.length} regression(s) vs '${compareName}':`);
        for (const regression of regressions) console.log(`  - ${regression}`);
        // Compare only fails the run under the strict enforce flag.
        if (ENFORCE) {
          expect(regressions, 'no configured regressions under strict enforce').toEqual([]);
        }
      }
    }
  });
});

function aggregate(
  scenario: string,
  boundaryCount: number,
  runs: number,
  cpu: number[],
  elapsed: number[],
  peak: Array<number | null>,
  retained: Array<number | null>,
  longTask: Array<number | null>,
): AggregatedRow {
  return {
    scenario,
    boundaryCount,
    runs,
    cpuMsMedian: median(cpu),
    cpuMsP95: p95(cpu),
    elapsedMsMedian: median(elapsed),
    elapsedMsP95: p95(elapsed),
    peakHeapDeltaBytesMedian: medianNullable(peak),
    retainedHeapDeltaBytesMedian: medianNullable(retained),
    maxLongTaskMsMedian: medianNullable(longTask),
    cpuSamples: cpu,
    elapsedSamples: elapsed,
    retainedSamples: retained,
    peakSamples: peak,
  };
}

/** Opt-in strict gates. Structured and documented rather than woven into the
 *  deterministic assertions, so flaky timing/heap noise never fails a normal
 *  run — only an explicit `WEBUI_STREAMING_HYDRATION_ENFORCE=1` run. */
function enforceStrictGates(rows: AggregatedRow[]): void {
  const control = rows.find((r) => r.boundaryCount === 0);
  const streaming = rows.filter((r) => r.boundaryCount > 0);
  const b1 = streaming.find((r) => r.boundaryCount === 1);
  expect(control, 'control row present').toBeTruthy();
  expect(b1, 'single-boundary streaming row present').toBeTruthy();
  if (!control || !b1) return;

  // (a) Component-hydration CPU: each multi-boundary arm stays within 5% of
  //     the same streaming activation path delivered as one boundary. The
  //     ordinary control uses a different bootstrap entry point and is retained
  //     for production-path/jank comparison, not as a permissive CPU baseline.
  const cpuAllowance = b1.cpuMsMedian * (CPU_TOLERANCE_PCT / 100) + CPU_NOISE_FLOOR_MS;
  const cpuCap = b1.cpuMsMedian + cpuAllowance;
  console.log(
    `\n[hydration] strict gates (n=${control.runs}): single-boundary streaming CPU ${b1.cpuMsMedian.toFixed(2)}ms`
    + ` -> CPU cap ${cpuCap.toFixed(2)}ms (+${CPU_TOLERANCE_PCT}% + ${CPU_NOISE_FLOOR_MS}ms floor)`,
  );
  for (const row of streaming) {
    const pairedDelta = median(
      row.cpuSamples.map((sample, index) => sample - b1.cpuSamples[index]),
    );
    expect(
      pairedDelta,
      `${row.scenario}: paired component CPU delta ${pairedDelta.toFixed(2)}ms <= ${cpuAllowance.toFixed(2)}ms`,
    ).toBeLessThanOrEqual(cpuAllowance);
  }

  // (b) Retained-heap slope across N=1/10/100 <= max(64KiB, 2%) after forced GC.
  const retainedByBoundary = new Map<number, number | null>();
  for (const row of streaming) retainedByBoundary.set(row.boundaryCount, row.retainedHeapDeltaBytesMedian);
  const r1 = retainedByBoundary.get(1);
  const r10 = retainedByBoundary.get(10);
  const r100 = retainedByBoundary.get(100);
  if (r1 !== null && r1 !== undefined && r10 !== null && r10 !== undefined && r100 !== null && r100 !== undefined) {
    const slope = Math.max(r1, r10, r100) - Math.min(r1, r10, r100);
    const cap = Math.max(RETAINED_SLOPE_ABS_BYTES, Math.abs(r1) * (RETAINED_SLOPE_PCT / 100));
    console.log(
      `[hydration] retained-heap slope ${(slope / 1024).toFixed(1)}KiB -> cap ${(cap / 1024).toFixed(1)}KiB`
      + ` (max(${(RETAINED_SLOPE_ABS_BYTES / 1024).toFixed(0)}KiB, ${RETAINED_SLOPE_PCT}%))`,
    );
    expect(
      slope,
      `retained-heap slope ${(slope / 1024).toFixed(1)}KiB <= ${(cap / 1024).toFixed(1)}KiB (N=1/10/100)`,
    ).toBeLessThanOrEqual(cap);
  }

  // (c) Peak heap must not grow with boundary count: every streaming arm's peak
  //     stays within max(512KiB, 15%) of the single-boundary (largest-boundary)
  //     working set. Referenced against streaming itself, since the coordinator
  //     carries transient scaffolding the one-shot control never allocates.
  const b1Peak = streaming.find((r) => r.boundaryCount === 1)?.peakHeapDeltaBytesMedian ?? null;
  if (b1Peak !== null) {
    const peakAllowance = Math.max(PEAK_HEAP_ABS_FLOOR_BYTES, b1Peak * (PEAK_HEAP_TOLERANCE_PCT / 100));
    const peakCap = b1Peak + peakAllowance;
    console.log(
      `[hydration] single-boundary peak ${(b1Peak / 1024).toFixed(1)}KiB`
      + ` -> peak cap ${(peakCap / 1024).toFixed(1)}KiB (+max(${(PEAK_HEAP_ABS_FLOOR_BYTES / 1024).toFixed(0)}KiB, ${PEAK_HEAP_TOLERANCE_PCT}%))`,
    );
    for (const row of streaming) {
      if (row.peakHeapDeltaBytesMedian === null) continue;
      expect(
        row.peakHeapDeltaBytesMedian,
        `${row.scenario}: peak heap ${(row.peakHeapDeltaBytesMedian / 1024).toFixed(1)}KiB <= ${(peakCap / 1024).toFixed(1)}KiB`,
      ).toBeLessThanOrEqual(peakCap);
    }
  }

  // (d) Linear coordinator elapsed growth: the observed marginal cost from the
  //     smallest to largest boundary count stays below a reviewed per-boundary
  //     cap. This catches quadratic marker rescans without tying the fixed
  //     component work to one machine's absolute speed.
  const b100 = streaming.find((r) => r.boundaryCount === 100);
  if (b100) {
    const pairedElapsedDelta = median(
      b100.elapsedSamples.map((sample, index) => sample - b1.elapsedSamples[index]),
    );
    const perBoundary = pairedElapsedDelta / (100 - 1);
    const perBoundaryCap = Math.max(
      COORDINATOR_MARGINAL_ABS_CAP_MS,
      b1.elapsedMsMedian * (COORDINATOR_MARGINAL_RELATIVE_PCT / 100),
    );
    console.log(
      `[hydration] coordinator marginal cost ${perBoundary.toFixed(3)}ms/boundary`
      + ` -> cap ${perBoundaryCap.toFixed(3)}ms`
      + ` (max(${COORDINATOR_MARGINAL_ABS_CAP_MS}ms, ${COORDINATOR_MARGINAL_RELATIVE_PCT}% of B=1 elapsed))`,
    );
    expect(
      perBoundary,
      `coordinator marginal cost ${perBoundary.toFixed(3)}ms/boundary <= ${perBoundaryCap.toFixed(3)}ms (linear growth)`,
    ).toBeLessThanOrEqual(perBoundaryCap);
  }
}
