// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import {
  BenchmarkInputError,
  buildBenchmarkView,
  DEFAULT_METRIC,
  formatMeasurement,
  METRIC_IDS,
  prepareBenchmarkEvidence,
} from '../../../docs/.webui-press/components/benchmark-explorer/benchmark-data.js';
import type {
  BenchmarkView,
  MetricId,
} from '../../../docs/.webui-press/components/benchmark-explorer/benchmark-data.js';
import { buildBenchmarkPreviews } from '../../../docs/.webui-press/components/benchmark-preview/benchmark-preview-data.js';
import type { BenchmarkPreview } from '../../../docs/.webui-press/components/benchmark-preview/benchmark-preview-data.js';
import { fixture, removeFixtureRoot, write } from './native-fixture.js';

const workspace = path.resolve(import.meta.dirname, '../../..');
const press = path.join(workspace, 'docs/.webui-press');
const sourceText = fs.readFileSync(path.join(press, 'state/benchmark-summary.json'), 'utf8');

interface HistoricalInput {
  benchmarks: {
    asyncDataBoundary: { canonicalWebuiVersion: string; status: string };
    metrics: { id: MetricId; unit: string; direction: string; methodology: string }[];
    rows: {
      caseId: string;
      label: string;
      metrics: Record<MetricId, { available: boolean; value: number | null; detail: string }>;
    }[];
    selectiveRefresh: { refreshedAt: string; caseIds: string[] };
  };
}

function source(): HistoricalInput {
  return JSON.parse(sourceText) as HistoricalInput;
}

test('homepage evidence defaults to RPS and retains full-view scales for four ordered metrics', () => {
  const evidence = prepareBenchmarkEvidence(source());
  const before = JSON.stringify(evidence);
  const previews = buildBenchmarkPreviews(evidence);
  assert.deepEqual(previews.map(preview => preview.id), [
    'noStreamingRequestsPerSecond', 'streamingRequestsPerSecond', 'jsHeapUsedBytes', 'rendererPrivateBytes',
  ]);
  assert.deepEqual(previews.map(preview => preview.label), ['RPS', 'Streaming', 'JS heap', 'Renderer memory']);
  assert.deepEqual(previews.map(preview => preview.selected), [true, false, false, false]);
  assert.deepEqual(previews.map(preview => preview.tabIndex), [0, -1, -1, -1]);
  assert.deepEqual(previews.map(preview => preview.samples), [1, 1, 10, 10]);
  assert.deepEqual(previews.map(preview => preview.maximumLabel), ['1,593.4', '1,388.45', '5.22', '73.31']);
  for (const preview of previews) {
    const view = buildBenchmarkView(evidence, preview.id);
    assert.deepEqual(preview.rows.map(row => row.caseId), ['webui-rust-actix', 'react', 'next']);
    assert.deepEqual(preview.rows.map(row => row.label), ['WebUI', 'React', 'Next.js']);
    assert.equal(view.rows.find(row => row.caseId === 'webui-rust-actix')?.label, 'WebUI / Light DOM / Rust / Actix');
    assert.equal(preview.unit, view.activeMetric.unit);
    assert.equal(preview.maximumLabel, view.maximumLabel);
    assert.equal(preview.midpointLabel, view.midpointLabel);
    assert.equal(preview.directionLabel, view.activeMetric.directionLabel);
    for (const row of preview.rows) {
      const full = view.rows.find(candidate => candidate.caseId === row.caseId);
      assert.ok(full);
      assert.equal(row.barWidth, full.barWidth);
      assert.equal(row.valueLabel, full.valueLabel);
      assert.ok(!('values' in row), 'do not copy all metrics or telemetry into each projected row');
    }
  }
  assert.deepEqual(previews.map(preview => preview.rows[0].barWidth), [
    100, 100, 40.24523190606928, 79.77354469162115,
  ]);
  assert.match(previews[0].samplingLabel, /1 measured round.*variability/);
  assert.match(previews[3].description, /private bytes, not RSS/);
  assert.equal(JSON.stringify(evidence), before);
  assert.equal(JSON.stringify(buildBenchmarkPreviews(evidence)), JSON.stringify(previews));
});

test('homepage evidence keeps the full scale when an omitted configuration has the largest value', () => {
  const evidence = prepareBenchmarkEvidence(source());
  const omitted = evidence.rows.find(row => row.caseId === 'webui-shadowdom-rust-actix');
  assert.ok(omitted);
  for (const metric of evidence.metrics) {
    const maximum = Math.max(...evidence.rows.map(row => row.values[metric.id].value ?? 0));
    omitted.values[metric.id].value = maximum * 2;
  }
  for (const preview of buildBenchmarkPreviews(evidence)) {
    assert.ok(preview.rows.every(row => row.caseId !== omitted.caseId));
    assert.ok(preview.rows.every(row => row.barWidth !== null && row.barWidth <= 50),
      'Do not rescale to the three featured configurations.');
    assert.equal(preview.maximumLabel, buildBenchmarkView(evidence, preview.id).maximumLabel);
  }
});

test('homepage evidence rejects missing configurations instead of substituting a better result', () => {
  for (const caseId of ['webui-rust-actix', 'react', 'next']) {
    const evidence = prepareBenchmarkEvidence(source());
    evidence.rows = evidence.rows.filter(row => row.caseId !== caseId);
    assert.throws(() => buildBenchmarkPreviews(evidence), error =>
      error instanceof BenchmarkInputError && error.code === 'benchmark-preview' && error.field === caseId);
  }
});

test('homepage evidence preserves unavailable and measured-zero values', () => {
  const evidence = prepareBenchmarkEvidence(source());
  const missing = evidence.rows.find(row => row.caseId === 'react');
  const zero = evidence.rows.find(row => row.caseId === 'next');
  assert.ok(missing && zero);
  for (const id of METRIC_IDS) {
    missing.values[id] = { value: null, display: 'Unavailable', detail: 'Not recorded.' };
    zero.values[id] = { value: 0, display: '0', detail: 'Synthetic measured zero.' };
  }
  for (const preview of buildBenchmarkPreviews(evidence)) {
    assert.equal(preview.rows[1].available, false);
    assert.equal(preview.rows[1].barWidth, null);
    assert.equal(preview.rows[1].valueLabel, 'Unavailable');
    assert.equal(preview.rows[2].zero, true);
    assert.equal(preview.rows[2].barWidth, 0);
    assert.equal(preview.rows[2].valueLabel, `0 ${preview.unit}`);
  }
});

test('historical evidence keeps every case and numeric value, with JS heap selected by default', () => {
  const original = source();
  const evidence = prepareBenchmarkEvidence(original);
  assert.equal(evidence.rows.length, 16);
  assert.deepEqual(evidence.metrics.map(metric => metric.id), METRIC_IDS);
  for (const oldRow of original.benchmarks.rows) {
    const row = evidence.rows.find(candidate => candidate.caseId === oldRow.caseId);
    assert.ok(row);
    assert.equal(row.label, oldRow.label);
    for (const id of METRIC_IDS) {
      assert.equal(row.values[id].value, oldRow.metrics[id].value);
      assert.equal(row.values[id].detail, oldRow.metrics[id].detail);
    }
  }
  const view = buildBenchmarkView(evidence);
  assert.equal(view.selectedMetric, DEFAULT_METRIC);
  assert.equal(view.activeMetric.label, 'JavaScript heap');
  assert.deepEqual(view.metrics.filter(metric => metric.selected).map(metric => metric.id), [DEFAULT_METRIC]);
  assert.equal(view.availableCount, 16);
  assert.equal(view.unavailableCount, 0);
  assert.equal(view.rows[0].caseId, 'webui-deno-serve');
  assert.equal(view.rows[1].caseId, 'webui-bun-serve');
  assert.equal(view.rows[2].caseId, 'webui-node-http');
  assert.equal(view.rows[0].valueLabel, view.rows[1].valueLabel, 'sorting uses raw values, not rounded labels');
});

test('each metric uses its own direction, zero baseline, and deterministic ties without mutating evidence', () => {
  const evidence = prepareBenchmarkEvidence(source());
  const before = JSON.stringify(evidence);
  for (const id of METRIC_IDS) {
    const view = buildBenchmarkView(evidence, id);
    const values = view.rows.map(row => row.values[id].value);
    const expected = [...values].sort((left, right) => {
      assert.ok(left !== null && right !== null);
      return view.activeMetric.direction === 'higher' ? right - left : left - right;
    });
    assert.deepEqual(values, expected);
    assert.equal(Math.max(...view.rows.map(row => row.barWidth!)), 100);
    for (const row of view.rows) {
      assert.equal(row.values, evidence.rows.find(item => item.caseId === row.caseId)?.values,
        'switching reuses the measurement records instead of cloning telemetry');
    }
    assert.equal(buildBenchmarkView(view, id).selectedMetric, id);
  }
  assert.equal(JSON.stringify(evidence), before);
  const first = evidence.rows[0];
  const second = evidence.rows[1];
  first.values.jsHeapUsedBytes.value = second.values.jsHeapUsedBytes.value;
  first.label = second.label = 'Same label';
  const tied = buildBenchmarkView(evidence).rows.filter(row => row.label === 'Same label');
  assert.deepEqual(tied.map(row => row.caseId), [first.caseId, second.caseId].sort());
});

test('units convert bytes to MiB only, preserve zero, and never round positive values to zero', () => {
  const { metrics } = prepareBenchmarkEvidence(source());
  for (const metric of metrics) {
    assert.equal(formatMeasurement(null, metric), 'Unavailable');
    assert.equal(formatMeasurement(0, metric), '0');
    if (metric.rawUnit === 'bytes') {
      assert.equal(metric.unit, 'MiB');
      assert.equal(formatMeasurement(1_048_576, metric), '1');
      assert.equal(formatMeasurement(1, metric), '<0.01');
      assert.equal(formatMeasurement(1_572_864, metric), '1.5');
    } else {
      assert.equal(formatMeasurement(1000.25, metric), '1,000.25');
    }
  }
});

test('unavailable values remain null and sort last, while measured zero has no minimum-width bar', () => {
  const input = source();
  const zeroId = input.benchmarks.rows[0].caseId;
  const missingId = input.benchmarks.rows[1].caseId;
  for (const id of METRIC_IDS) {
    input.benchmarks.rows[0].metrics[id].value = 0;
    input.benchmarks.rows[1].metrics[id] = {
      available: false, value: null, detail: 'Not recorded in this synthetic validation case.',
    };
  }
  const evidence = prepareBenchmarkEvidence(input);
  for (const id of METRIC_IDS) {
    const view = buildBenchmarkView(evidence, id);
    const zero = view.rows.find(row => row.caseId === zeroId);
    assert.ok(zero);
    assert.equal(zero.barWidth, 0);
    assert.equal(zero.zero, true);
    assert.equal(zero.available, true);
    assert.equal(view.rows.at(-1)?.caseId, missingId);
    assert.equal(view.rows.at(-1)?.barWidth, null);
    assert.equal(view.rows.at(-1)?.valueLabel, 'Unavailable');
    assert.equal(view.rows.at(-1)?.zero, false);
    assert.equal(view.unavailableCount, 1);
  }
});

test('an empty scale is unavailable, and an all-zero scale is a measured zero', () => {
  const input = source();
  for (const row of input.benchmarks.rows) {
    row.metrics.jsHeapUsedBytes = { available: false, value: null, detail: 'Not recorded.' };
  }
  const missing = buildBenchmarkView(prepareBenchmarkEvidence(input));
  assert.equal(missing.maximumLabel, 'Unavailable');
  assert.equal(missing.midpointLabel, 'Unavailable');
  assert.equal(missing.scaleAvailable, false);
  assert.equal(missing.unavailableCount, 16);
  assert.ok(missing.rows.every(row => row.barWidth === null));
  for (const row of input.benchmarks.rows) {
    row.metrics.jsHeapUsedBytes = {
      available: true, value: 0, detail: '10 isolated runs · synthetic zero-value validation case',
    };
  }
  const zero = buildBenchmarkView(prepareBenchmarkEvidence(input));
  assert.equal(zero.maximumLabel, '0');
  assert.equal(zero.scaleAvailable, true);
  assert.ok(zero.rows.every(row => row.available && row.zero && row.barWidth === 0));
});

test('sanitized provenance preserves sampling and mixed refresh, without winner or private-source metadata', () => {
  const evidence = prepareBenchmarkEvidence(source());
  assert.equal(evidence.provenance.serverMeasuredRounds, 1);
  assert.equal(evidence.provenance.browserIsolatedRuns, 10);
  assert.deepEqual(evidence.rows.filter(row => row.refreshed).map(row => row.caseId), [
    'webui-bun-serve', 'webui-deno-serve', 'webui-node-http',
  ]);
  assert.equal(evidence.provenance.selectiveRefresh.refreshedAt, '2026-09-04T03:28:11.463Z');
  assert.match(evidence.provenance.sourceAccess, /HTTP 404 without authentication/);
  assert.match(evidence.provenance.limitations.join('\n'), /0\.0\.28.*capability metadata only/);
  assert.match(evidence.provenance.limitations.join('\n'), /not a complete reproducibility package/);
  const json = JSON.stringify(evidence);
  for (const removed of ['benchmarksJson', 'serverTelemetry', 'baseCommit', 'refreshedCommit', 'metricSummary', '399.0%']) {
    assert.ok(!json.includes(removed), `${removed} should not be published`);
  }
  assert.ok(json.length < sourceText.length / 5, 'do not hydrate the original bulk telemetry and duplicated JSON');
});

const invalidCases: [string, string, (input: HistoricalInput) => void][] = [
  ['missing measurement', 'benchmark-object', input => { delete (input.benchmarks.rows[0].metrics as Partial<Record<MetricId, unknown>>).jsHeapUsedBytes; }],
  ['missing raw value', 'benchmark-value', input => { delete (input.benchmarks.rows[0].metrics.jsHeapUsedBytes as { value?: unknown }).value; }],
  ['negative value', 'benchmark-value', input => { input.benchmarks.rows[0].metrics.jsHeapUsedBytes.value = -1; }],
  ['non-finite value', 'benchmark-value', input => { input.benchmarks.rows[0].metrics.jsHeapUsedBytes.value = Infinity; }],
  ['available null', 'benchmark-value', input => { input.benchmarks.rows[0].metrics.jsHeapUsedBytes.value = null; }],
  ['unavailable zero', 'benchmark-unavailable', input => { input.benchmarks.rows[0].metrics.jsHeapUsedBytes = { available: false, value: 0, detail: 'Missing' }; }],
  ['missing detail', 'benchmark-text', input => { input.benchmarks.rows[0].metrics.jsHeapUsedBytes.detail = ''; }],
  ['changed units', 'benchmark-unit', input => { input.benchmarks.metrics[0].unit = 'MB'; }],
  ['changed direction', 'benchmark-unit', input => { input.benchmarks.metrics[0].direction = 'lower'; }],
  ['missing row', 'benchmark-count', input => { input.benchmarks.rows.pop(); }],
  ['duplicate row', 'benchmark-case', input => { input.benchmarks.rows[1].caseId = input.benchmarks.rows[0].caseId; }],
  ['missing metric', 'benchmark-count', input => { input.benchmarks.metrics.pop(); }],
  ['duplicate metric', 'benchmark-metric', input => { input.benchmarks.metrics[1].id = input.benchmarks.metrics[0].id; }],
  ['different sampling', 'benchmark-sampling', input => { input.benchmarks.metrics[0].methodology = '7 measured rounds'; }],
  ['different browser samples', 'benchmark-sampling', input => { input.benchmarks.rows[0].metrics.jsHeapUsedBytes.detail = '7 runs'; }],
  ['unrecorded refresh date', 'benchmark-refresh', input => { input.benchmarks.selectiveRefresh.refreshedAt = '2026-09-05'; }],
  ['changed refresh cases', 'benchmark-refresh', input => { input.benchmarks.selectiveRefresh.caseIds[0] = 'react'; }],
  ['capability is not a measured version', 'benchmark-capability', input => { input.benchmarks.asyncDataBoundary.canonicalWebuiVersion = '3.0.4'; }],
];

for (const [name, code, mutate] of invalidCases) {
  test(`evidence preparation fails actionably for ${name}`, () => {
    const input = source();
    mutate(input);
    assert.throws(() => prepareBenchmarkEvidence(input), error =>
      error instanceof BenchmarkInputError && error.code === code && error.message.includes('help:'));
  });
}

test('evidence and initial view are deterministic even when input rows and metric definitions are reordered', () => {
  const input = source();
  const expected = prepareBenchmarkEvidence(input);
  input.benchmarks.rows.reverse();
  input.benchmarks.metrics.reverse();
  assert.equal(JSON.stringify(prepareBenchmarkEvidence(input)), JSON.stringify(expected));
  assert.equal(JSON.stringify(buildBenchmarkView(prepareBenchmarkEvidence(input))), JSON.stringify(buildBenchmarkView(expected)));
});

test('the real preparation script emits deterministic ignored assets and fails instead of substituting data', t => {
  const f = fixture();
  t.after(() => removeFixtureRoot(f.root));
  const local = path.join(f.site, '.webui-press');
  for (const relative of [
    'scripts/prepare-benchmarks.mjs',
    'components/benchmark-explorer/benchmark-data.ts',
    'components/benchmark-preview/benchmark-preview-data.ts',
  ]) {
    write(path.join(local, relative), fs.readFileSync(path.join(press, relative), 'utf8'));
  }
  const input = path.join(local, 'state/benchmark-summary.json');
  write(input, sourceText);
  const script = path.join(local, 'scripts/prepare-benchmarks.mjs');
  const args = process.allowedNodeEnvironmentFlags.has('--no-experimental-strip-types')
    ? ['--no-experimental-strip-types', script]
    : [script];
  const run = () => execFileSync(process.execPath, args, {
    cwd: f.site, encoding: 'utf8', stdio: 'pipe',
  });
  run();
  const compiledFile = path.join(local, 'generated/benchmark-data.mjs');
  const compiledBytes = fs.readFileSync(compiledFile, 'utf8');
  const compiledPreview = path.join(local, 'generated/benchmark-preview-data.mjs');
  const previewBytes = fs.readFileSync(compiledPreview, 'utf8');
  const stateFile = path.join(local, 'generated/benchmark-state.json');
  const homeFile = path.join(local, 'generated/home-benchmark-state.json');
  const downloadFile = path.join(local, 'public/benchmarks/historical-results.json');
  const stateBytes = fs.readFileSync(stateFile, 'utf8');
  const homeBytes = fs.readFileSync(homeFile, 'utf8');
  const downloadBytes = fs.readFileSync(downloadFile, 'utf8');
  const state = JSON.parse(stateBytes) as { benchmarks: BenchmarkView };
  const home = JSON.parse(homeBytes) as {
    previewMetrics: BenchmarkPreview[]; previewCaseCount: number; previewTotalCases: number;
  };
  assert.equal(state.benchmarks.selectedMetric, DEFAULT_METRIC);
  assert.deepEqual(Object.keys(state), ['benchmarks']);
  assert.deepEqual(Object.keys(home), ['previewMetrics', 'previewCaseCount', 'previewTotalCases']);
  assert.equal(home.previewCaseCount, 3);
  assert.equal(home.previewTotalCases, 16);
  assert.deepEqual(home.previewMetrics, buildBenchmarkPreviews(state.benchmarks));
  assert.ok(homeBytes.length < 4096, 'four three-case previews must stay below 4 KiB of generated JSON');
  assert.deepEqual(JSON.parse(downloadBytes), prepareBenchmarkEvidence(source()));
  run();
  assert.equal(fs.readFileSync(compiledFile, 'utf8'), compiledBytes);
  assert.equal(fs.readFileSync(compiledPreview, 'utf8'), previewBytes);
  assert.equal(fs.readFileSync(stateFile, 'utf8'), stateBytes);
  assert.equal(fs.readFileSync(homeFile, 'utf8'), homeBytes);
  assert.equal(fs.readFileSync(downloadFile, 'utf8'), downloadBytes);
  const helper = path.join(local, 'components/benchmark-explorer/benchmark-data.ts');
  write(helper, 'export const broken = ;');
  const compilationFailure = spawnSync(process.execPath, args, {
    cwd: f.site, encoding: 'utf8',
  });
  assert.equal(compilationFailure.status, 1);
  assert.match(compilationFailure.stderr, /\[benchmark-compile\].*Cannot compile/s);
  assert.match(compilationFailure.stderr, /help:/);
  assert.equal(fs.readFileSync(stateFile, 'utf8'), stateBytes, 'a failed compilation must not publish stale helper results');
  write(helper, fs.readFileSync(path.join(press, 'components/benchmark-explorer/benchmark-data.ts'), 'utf8'));
  write(input, '{"benchmarks":');
  const failure = spawnSync(process.execPath, args, {
    cwd: f.site, encoding: 'utf8',
  });
  assert.equal(failure.status, 1);
  assert.match(failure.stderr, /\[benchmark-read\].*valid JSON/s);
  assert.match(failure.stderr, /help:/);
  assert.equal(fs.readFileSync(stateFile, 'utf8'), stateBytes, 'a failed preparation must not replace valid output with fallbacks');
  assert.equal(fs.readFileSync(homeFile, 'utf8'), homeBytes, 'failed preparation must retain the valid preview too');
  const ignore = fs.readFileSync(path.join(workspace, 'docs/.gitignore'), 'utf8');
  assert.ok(ignore.includes('.webui-press/generated/'));
  assert.ok(ignore.includes('.webui-press/public/benchmarks/'));
});
