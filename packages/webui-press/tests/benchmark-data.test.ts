// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import {
  BenchmarkInputError,
  buildBenchmarkView,
  DEFAULT_METRIC,
  formatMeasurement,
  METRIC_IDS,
  prepareBenchmarkCampaignDigestPayload,
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

interface CampaignInput {
  benchmarksJson: string;
  benchmarks: {
    asyncDataBoundary: { canonicalWebuiVersion: string; status: string };
    metrics: { id: MetricId; unit: string; direction: string; methodology: string }[];
    rows: {
      caseId: string;
      label: string;
      metrics: Record<MetricId, { available: boolean; value: number | null; detail: string }>;
    }[];
    campaign: {
      canonicalWebuiVersion: string;
      sourceCommit: string;
      sourceDigest: string;
      fixtureDigest: string;
      evidenceDigest: string;
      responseModes: string[];
      uniformOfficialMatrix: boolean;
      workingTreeDirty: boolean;
      rowCount: number;
      serverMeasuredRounds: number;
      browserRoundsPerApp: number;
      progressiveMeasuredAt: string;
      completeMeasuredAt: string;
      browserMeasuredAt: string;
    };
  };
}

function source(): CampaignInput {
  return JSON.parse(sourceText) as CampaignInput;
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
  assert.deepEqual(previews.map(preview => preview.maximumLabel), ['1,741.15', '1,569.3', '4.94', '57.01']);
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
    100, 100, 43.281342630766886, 74.3713600548133,
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

test('campaign input keeps its serialized mirror aligned', () => {
  const original = source();
  assert.deepEqual(JSON.parse(original.benchmarksJson), original.benchmarks);
});

test('campaign evidence digest is pinned to the reviewed canonical measurements', () => {
  const original = source();
  const evidence = prepareBenchmarkEvidence(original);
  const digest = createHash('sha256')
    .update(JSON.stringify(prepareBenchmarkCampaignDigestPayload(evidence)), 'utf8')
    .digest('hex');
  assert.equal(digest, 'fd005660c1af487600a266a0669e905e54cbad73a9e075608ca8a676afd9fdd3');
  assert.equal(original.benchmarks.campaign.evidenceDigest, digest);
  for (const row of evidence.rows) {
    for (const id of METRIC_IDS) {
      const measurement = row.values[id];
      const before = { ...measurement };
      for (const change of [
        () => { measurement.value = (before.value ?? 0) + 1; },
        () => { measurement.detail += ' Changed note.'; },
      ]) {
        change();
        const changedDigest = createHash('sha256')
          .update(JSON.stringify(prepareBenchmarkCampaignDigestPayload(evidence)), 'utf8')
          .digest('hex');
        assert.notEqual(changedDigest, digest, `${row.caseId}.${id} values and notes must be bound`);
        Object.assign(measurement, before);
      }
    }
  }
  assert.equal(JSON.stringify(original), JSON.stringify(source()), 'canonicalization must not mutate input');
});

test('campaign evidence preserves all official configurations and numeric values, with JS heap selected by default', () => {
  const original = source();
  const evidence = prepareBenchmarkEvidence(original);
  assert.deepEqual(evidence.rows.map(row => row.caseId), [
    'astro', 'lit', 'next', 'nuxt', 'preact', 'react', 'react-tanstack', 'solid', 'sveltekit', 'vue',
    'webui-bun-serve', 'webui-fast-rust-actix', 'webui-node-http',
    'webui-rust-actix', 'webui-shadowdom-rust-actix',
  ]);
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
  assert.equal(view.availableCount, 15);
  assert.equal(view.unavailableCount, 0);
  assert.equal(view.rows[0].caseId, 'webui-bun-serve');
  assert.equal(view.rows[1].caseId, 'webui-node-http');
  assert.equal(view.rows[2].caseId, 'webui-rust-actix');
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
  assert.equal(missing.unavailableCount, 15);
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

test('sanitized provenance preserves the uniform campaign without winner or bulk telemetry', () => {
  const evidence = prepareBenchmarkEvidence(source());
  assert.equal(evidence.provenance.serverMeasuredRounds, 1);
  assert.equal(evidence.provenance.browserIsolatedRuns, 10);
  const campaign = source().benchmarks.campaign;
  assert.equal(evidence.provenance.campaign.sourceCommit, campaign.sourceCommit);
  assert.equal(evidence.provenance.campaign.sourceDigest, campaign.sourceDigest);
  assert.equal(evidence.provenance.campaign.fixtureDigest, campaign.fixtureDigest);
  assert.equal(evidence.provenance.campaign.canonicalWebuiVersion, '3.0.4');
  assert.equal(evidence.provenance.campaign.browserMeasuredAt, campaign.browserMeasuredAt);
  assert.deepEqual(evidence.provenance.campaign.responseModes, ['complete', 'progressive']);
  assert.match(evidence.provenance.limitations.join('\n'), /unavailable.*sample counts/);
  assert.match(evidence.provenance.limitations.join('\n'), /not a complete reproducibility package/);
  const json = JSON.stringify(evidence);
  for (const removed of ['benchmarksJson', 'serverTelemetry', 'baseCommit', 'refreshedCommit', 'metricSummary', '399.0%']) {
    assert.ok(!json.includes(removed), `${removed} should not be published`);
  }
  assert.ok(json.length < sourceText.length / 5, 'do not hydrate the original bulk telemetry and duplicated JSON');
});

test('renderer-private notes retain partial sample counts and unavailability', () => {
  const evidence = prepareBenchmarkEvidence(source());
  const bun = evidence.rows.find(row => row.caseId === 'webui-bun-serve');
  assert.ok(bun);
  assert.match(bun.values.rendererPrivateBytes.detail, /^3 isolated runs .*7 unavailable$/);
  assert.equal(bun.values.rendererPrivateBytes.value, 40861696);
});

const invalidCases: [string, string, (input: CampaignInput) => void][] = [
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
  ['unknown case', 'benchmark-case', input => { input.benchmarks.rows[0].caseId = 'webui-deno-serve'; }],
  ['nonuniform campaign', 'benchmark-campaign', input => { input.benchmarks.campaign.uniformOfficialMatrix = false; }],
  ['wrong campaign row count', 'benchmark-campaign', input => { input.benchmarks.campaign.rowCount = 14; }],
  ['dirty campaign revision', 'benchmark-campaign', input => { input.benchmarks.campaign.workingTreeDirty = true; }],
  ['wrong server sampling', 'benchmark-campaign', input => { input.benchmarks.campaign.serverMeasuredRounds = 2; }],
  ['wrong browser sampling', 'benchmark-campaign', input => { input.benchmarks.campaign.browserRoundsPerApp = 3; }],
  ['missing response mode', 'benchmark-count', input => { input.benchmarks.campaign.responseModes.pop(); }],
  ['wrong response modes', 'benchmark-campaign', input => { input.benchmarks.campaign.responseModes.reverse(); }],
  ['invalid source commit', 'benchmark-campaign', input => { input.benchmarks.campaign.sourceCommit = 'invalid'; }],
  ['mismatched digest', 'benchmark-campaign', input => { input.benchmarks.campaign.fixtureDigest = 'a'.repeat(64); }],
  ['invalid evidence digest', 'benchmark-campaign', input => { input.benchmarks.campaign.evidenceDigest = 'invalid'; }],
  ['invalid timestamp', 'benchmark-campaign', input => { input.benchmarks.campaign.browserMeasuredAt = 'invalid'; }],
  ['mismatched capability version', 'benchmark-capability', input => { input.benchmarks.asyncDataBoundary.canonicalWebuiVersion = '0.0.28'; }],
  ['undisclosed missing samples', 'benchmark-sampling', input => { input.benchmarks.rows[0].metrics.rendererPrivateBytes.detail = '3 isolated runs · MAD 0 MiB'; }],
  ['too many samples', 'benchmark-sampling', input => { input.benchmarks.rows[0].metrics.rendererPrivateBytes.detail = '11 isolated runs · MAD 0 MiB'; }],
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
  const reordered = prepareBenchmarkEvidence(input);
  assert.equal(JSON.stringify(reordered), JSON.stringify(expected));
  assert.deepEqual(prepareBenchmarkCampaignDigestPayload(reordered), prepareBenchmarkCampaignDigestPayload(expected));
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
  assert.match(run(), /Prepared official benchmark state and sanitized JSON \(15 cases, 5 metrics\)\./);
  const compiledFile = path.join(local, 'generated/benchmark-data.mjs');
  const compiledBytes = fs.readFileSync(compiledFile, 'utf8');
  const compiledPreview = path.join(local, 'generated/benchmark-preview-data.mjs');
  const previewBytes = fs.readFileSync(compiledPreview, 'utf8');
  const stateFile = path.join(local, 'generated/benchmark-state.json');
  const homeFile = path.join(local, 'generated/home-benchmark-state.json');
  const downloadFile = path.join(local, 'public/benchmarks/official-results.json');
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
  assert.equal(home.previewTotalCases, 15);
  assert.deepEqual(home.previewMetrics, buildBenchmarkPreviews(state.benchmarks));
  assert.ok(homeBytes.length < 4096, 'four three-case previews must stay below 4 KiB of generated JSON');
  assert.deepEqual(JSON.parse(downloadBytes), prepareBenchmarkEvidence(source()));
  run();
  assert.equal(fs.readFileSync(compiledFile, 'utf8'), compiledBytes);
  assert.equal(fs.readFileSync(compiledPreview, 'utf8'), previewBytes);
  assert.equal(fs.readFileSync(stateFile, 'utf8'), stateBytes);
  assert.equal(fs.readFileSync(homeFile, 'utf8'), homeBytes);
  assert.equal(fs.readFileSync(downloadFile, 'utf8'), downloadBytes);
  const tamperingCases: [string, (input: CampaignInput) => void][] = [
    ['measurement', value => { value.benchmarks.rows[0].metrics.jsHeapUsedBytes.value = 123456789; }],
    ['note', value => { value.benchmarks.rows[0].metrics.jsHeapUsedBytes.detail += ' Changed note.'; }],
    ['availability', value => {
      value.benchmarks.rows[0].metrics.jsHeapUsedBytes.available = false;
      value.benchmarks.rows[0].metrics.jsHeapUsedBytes.value = null;
    }],
    ['label', value => { value.benchmarks.rows[0].label += ' Changed label'; }],
    ['methodology', value => { value.benchmarks.metrics[0].methodology += ' Changed methodology.'; }],
    ['source revision', value => { value.benchmarks.campaign.sourceCommit = 'a'.repeat(40); }],
    ['timestamp', value => { value.benchmarks.campaign.browserMeasuredAt = '2026-10-23T00:00:00.000Z'; }],
    ['matching source and fixture digests', value => {
      value.benchmarks.campaign.sourceDigest = value.benchmarks.campaign.fixtureDigest = 'b'.repeat(64);
    }],
    ['self-consistent but unreviewed evidence digest', value => {
      value.benchmarks.rows[0].metrics.jsHeapUsedBytes.value = 123456789;
      value.benchmarks.campaign.evidenceDigest = createHash('sha256')
        .update(JSON.stringify(prepareBenchmarkCampaignDigestPayload(prepareBenchmarkEvidence(value))), 'utf8')
        .digest('hex');
    }],
  ];
  for (const [name, mutate] of tamperingCases) {
    const changed = source();
    mutate(changed);
    changed.benchmarksJson = JSON.stringify(changed.benchmarks);
    write(input, JSON.stringify(changed));
    const failure = spawnSync(process.execPath, args, { cwd: f.site, encoding: 'utf8' });
    assert.equal(failure.status, 1, `reject changed ${name}`);
    assert.match(failure.stderr, /\[benchmark-digest\]/, name);
    assert.match(failure.stderr, /help:/, name);
    for (const [file, bytes] of [[stateFile, stateBytes], [homeFile, homeBytes], [downloadFile, downloadBytes]]) {
      assert.equal(fs.readFileSync(file, 'utf8'), bytes, `changed ${name} must not replace ${file}`);
    }
  }
  const reordered = source();
  reordered.benchmarks.rows.reverse();
  reordered.benchmarks.metrics.reverse();
  reordered.benchmarksJson = JSON.stringify(reordered.benchmarks);
  write(input, JSON.stringify(reordered));
  run();
  assert.equal(fs.readFileSync(downloadFile, 'utf8'), downloadBytes, 'input order must not change the canonical digest');
  write(input, sourceText);
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
