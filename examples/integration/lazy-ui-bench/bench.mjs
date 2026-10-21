// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { cpus, platform, release } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { esbuildVersion, modes, startFixtures } from './fixture.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..', '..');
const require = createRequire(join(root, 'package.json'));
const { chromium } = require('@playwright/test');
const samples = Number(process.env.WEBUI_BENCH_SAMPLES ?? 20);
const latencyMs = Number(process.env.WEBUI_BENCH_LATENCY_MS ?? 40);
const counts = [25, 250, 1000];
assert.ok(Number.isSafeInteger(samples) && samples >= 3 && samples <= 100);
assert.ok(Number.isFinite(latencyMs) && latencyMs >= 0 && latencyMs <= 1000);
assert.ok(process.env.WEBUI_ADDON_PATH, 'Set WEBUI_ADDON_PATH to the freshly built release addon.');

function stats(values) {
  const sorted = [...values].sort((a, b) => a - b);
  const median = sorted[Math.floor(sorted.length / 2)];
  const p90 = sorted[Math.ceil(sorted.length * 0.9) - 1];
  return { median, p90, min: sorted[0], max: sorted.at(-1), samples: values };
}

async function heap(session) {
  await session.send('HeapProfiler.collectGarbage');
  return (await session.send('Runtime.getHeapUsage')).usedSize;
}

async function correctness(page, count) {
  const content = await page.locator('bench-owner').evaluate(element => {
    const inputs = element.shadowRoot.querySelectorAll('input');
    return {
      count: inputs.length,
      values: Array.from(inputs, input => input.value),
      labels: Array.from(element.shadowRoot.querySelectorAll('label'), label => label.textContent),
    };
  });
  assert.equal(content.count, count);
  assert.deepEqual(content.values, Array(count).fill('Fresh'));
  assert.deepEqual(content.labels, Array.from({ length: count }, (_, i) => `Setting ${i}`));
}

const server = await startFixtures(counts, latencyMs);
let browser;
const rows = new Map();
try {
  browser = await chromium.launch();
  for (let iteration = -2; iteration < samples; iteration++) {
    const order = iteration % 2 === 0 ? modes : [...modes].reverse();
    for (const count of counts) for (const mode of order) {
      const context = await browser.newContext();
      const page = await context.newPage();
      const session = await context.newCDPSession(page);
      const errors = [];
      page.on('pageerror', error => errors.push(String(error)));
      const responses = [];
      page.on('response', response => {
        if (response.url().endsWith('/state')) responses.push(response);
      });
      try {
        await session.send('Performance.enable');
        const base = `/${mode}/${count}/`;
        await page.goto(server.origin + base);
        await page.waitForFunction(() => window.bench && document.querySelector('bench-owner').$ready);
        assert.equal(await page.locator('bench-owner input').count(), 0);
        const activationMs = await page.evaluate(() => window.bench.activationMs);
        const before = await heap(session);
        const openMs = await page.evaluate(() => window.bench.open());
        await correctness(page, count);
        await page.evaluate(() => window.bench.track());
        const openHeap = await heap(session);
        await page.evaluate(() => { window.bench.close(); window.bench.collect(); });
        assert.equal(await page.locator('bench-owner input').count(), 0);
        const collectedHeap = await heap(session);
        const retained = await page.evaluate(() => window.bench.retained());
        // Collection deliberately invalidates inventory; the next open is cold again.
        await page.evaluate(() => window.bench.open());
        await correctness(page, count);
        await page.evaluate(() => window.bench.close());
        const warmMs = await page.evaluate(() => window.bench.open());
        await correctness(page, count);
        const bodies = await Promise.all(responses.map(response => response.body()));
        if (mode === 'lazy') {
          assert.equal(responses.length, 3);
          const cold = JSON.parse(bodies[0]);
          const warm = JSON.parse(bodies[2]);
          assert.ok(cold.templates?.['bench-owner::lazy:panel']);
          assert.equal(warm.templates, undefined);
          assert.equal(warm.state.value, 'Fresh');
        }
        assert.deepEqual(errors, []);
        if (iteration >= 0) {
          let row = rows.get(base);
          if (!row) {
            const fixture = server.fixtures.get(base);
            row = {
              mode, controls: count,
              initialHtmlBytes: fixture.document.length,
              initialJsBytes: fixture.script.length,
              initialGzipBytes: fixture.gzipDocument.length + fixture.gzipScript.length,
              coldResponseBytes: bodies[0]?.length ?? 0,
              warmResponseBytes: bodies[2]?.length ?? 0,
              activationMs: [], initialHeapBytes: [], openMs: [], openHeapBytes: [],
              collectedHeapBytes: [], warmMs: [], closedBodyRetained: [],
            };
            rows.set(base, row);
          }
          row.activationMs.push(activationMs);
          row.initialHeapBytes.push(before);
          row.openMs.push(openMs);
          row.openHeapBytes.push(openHeap);
          row.collectedHeapBytes.push(collectedHeap);
          row.warmMs.push(warmMs);
          row.closedBodyRetained.push(retained);
        }
      } finally {
        await session.detach();
        await context.close();
      }
    }
  }
  const results = Array.from(rows.values(), row => {
    const result = { ...row };
    for (const key of ['activationMs', 'initialHeapBytes', 'openMs', 'openHeapBytes', 'collectedHeapBytes', 'warmMs']) {
      result[key] = stats(row[key]);
    }
    return result;
  });
  const report = {
    environment: {
      node: process.version, chromium: browser.version(), esbuild: esbuildVersion,
      os: `${platform()} ${release()}`, cpu: cpus()[0].model,
      samples, discardedWarmups: 2, stateDelayMs: latencyMs,
      addon: process.env.WEBUI_ADDON_PATH,
    },
    methodology: 'Fresh browser contexts, gzip HTML/JS, real compiler/protocol, production-minified bundles. Alternating mode order, 2 warmups, forced-GC heap samples. Eager-local keeps initial state; eager-fresh and lazy both fetch fresh state with identical provider delay. No physical low-end-device claim.',
    results,
  };
  const output = resolve(process.argv[2] ?? join(root, 'target', 'lazy-browser-proof.json'));
  await writeFile(output, JSON.stringify(report, null, 2) + '\n');
  console.table(results.map(row => ({
    mode: row.mode, controls: row.controls, initialGzip: row.initialGzipBytes,
    heap: row.initialHeapBytes.median, activateMs: row.activationMs.median,
    openMs: row.openMs.median, warmMs: row.warmMs.median,
    closedHeap: row.collectedHeapBytes.median,
    retainedRuns: row.closedBodyRetained.filter(Boolean).length,
  })));
  console.log(output);
} finally {
  await browser?.close();
  await server.close();
}
