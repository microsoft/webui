// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import {
  buildBenchmarkView,
  prepareBenchmarkEvidence,
} from '../../../docs/.webui-press/components/benchmark-explorer/benchmark-data.js';
import { build, fixture, removeFixtureRoot, write } from './native-fixture.js';

const workspace = path.resolve(import.meta.dirname, '../../..');
const docs = path.join(workspace, 'docs');
const component = path.join(docs, '.webui-press/components/benchmark-explorer');

test('the benchmark template server-renders every SVG and all five table metrics with page-local state', t => {
  const f = fixture();
  t.after(() => removeFixtureRoot(f.root));
  write(f.configFile, JSON.stringify({ ...f.config, regions: {} }));
  for (const file of fs.readdirSync(component)) {
    write(path.join(f.site, 'content/local/benchmark-explorer', file), fs.readFileSync(path.join(component, file), 'utf8'));
  }
  const evidence = prepareBenchmarkEvidence(JSON.parse(fs.readFileSync(
    path.join(docs, '.webui-press/state/benchmark-summary.json'), 'utf8',
  )));
  write(path.join(f.site, 'target/benchmark-state.json'), JSON.stringify({ benchmarks: buildBenchmarkView(evidence) }));
  write(path.join(f.site, 'content/benchmarks.md'), `---
title: Benchmark results
description: Historical measurements with explicit limits.
layout: page
stateFile: ../target/benchmark-state.json
---

# Measurements, with their limits

<benchmark-explorer :benchmarks="{{benchmarks}}"></benchmark-explorer>
`);
  build(f.site);
  const html = fs.readFileSync(path.join(f.site, 'dist/benchmarks/index.html'), 'utf8');
  const decodedSlashes = html.replaceAll('&#x2F;', '/');
  assert.match(html, /data-layout="page"/);
  assert.match(html, /Historical measurements with explicit limits/);
  assert.equal((html.match(/class="benchmark-plot"/g) ?? []).length, 14);
  assert.equal((html.match(/class="benchmark-row"/g) ?? []).length, 14);
  assert.equal((html.match(/<rect class="benchmark-bar"/g) ?? []).length, 14);
  assert.match(html, /<svg[^>]*role="img"/);
  const titles = [...decodedSlashes.matchAll(/<title[^>]*>(.*?)<\/title>/gs)]
    .map(match => match[1].replace(/<!--.*?-->/gs, ''));
  assert.ok(titles.includes('WebUI / Light DOM / Deno.serve: 2.1 MiB'));
  assert.match(html, /All bars start at zero/);
  assert.match(html, /<caption>Historical results/);
  assert.match(html, /aria-pressed="true"/);
  assert.match(html, /data-metric="jsHeapUsedBytes"[^>]*disabled/);
  for (const row of evidence.rows) assert.ok(decodedSlashes.includes(row.label));
  for (const row of evidence.rows) {
    for (const value of Object.values(row.values)) {
      assert.ok(html.includes(value.display), `missing table value ${value.display}`);
    }
  }
  assert.match(html, /Historical, limited, mixed-refresh data/);
  assert.match(html, /Browser metrics: 10 isolated runs\. Server metrics: 1 measured round/);
  assert.match(html, /benchmarks\/historical-results\.json/);
  assert.ok(!html.includes('serverTelemetry'));
  assert.ok(!html.includes('benchmarksJson'));
  const otherPage = fs.readFileSync(path.join(f.site, 'dist/page/index.html'), 'utf8');
  assert.ok(!otherPage.includes('"benchmarks":'), 'benchmark state must not leak onto ordinary pages');
});

test('the authored page has correct metadata, a wide layout, a generated local state file, and honest provenance', () => {
  const page = fs.readFileSync(path.join(docs, 'benchmarks.md'), 'utf8');
  assert.match(page, /title: Benchmark results/);
  assert.match(page, /description: .*historical/);
  assert.match(page, /layout: page/);
  assert.match(page, /stateFile: \.\/\.webui-press\/generated\/benchmark-state\.json/);
  assert.match(page, /:benchmarks="{{benchmarks}}"/);
  assert.match(page, /HTTP 404 without authentication/);
  assert.match(page, /one measured round/);
  assert.match(page, /10 isolated runs/);
  assert.match(page, /2026-09-04/);
  assert.match(page, /0\.0\.28/);
  assert.match(page, /not a complete reproducibility package/);
  const source = fs.readFileSync(path.join(component, 'benchmark-explorer.ts'), 'utf8');
  assert.ok(!source.includes('createElement'));
  assert.ok(!source.includes('querySelector'));
  assert.match(source, /buildBenchmarkView/);
  const scripts = JSON.parse(fs.readFileSync(path.join(docs, 'package.json'), 'utf8')) as {
    scripts: Record<string, string>;
  };
  assert.equal(scripts.scripts['prepare:benchmarks'], 'node .webui-press/scripts/prepare-benchmarks.mjs');
  assert.match(scripts.scripts.build, /prepare:benchmarks && webui press build/);
  assert.match(scripts.scripts.start, /prepare:benchmarks && webui press serve/);
});
