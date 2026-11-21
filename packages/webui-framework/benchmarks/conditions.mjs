// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createRequire } from 'node:module';
import { relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';

const pkg = fileURLToPath(new URL('..', import.meta.url));
const root = resolve(pkg, '..', '..');
const require = createRequire(resolve(pkg, 'package.json'));
const support = createRequire(resolve(pkg, '..', 'webui-test-support', 'package.json'));
const { chromium } = require('@playwright/test');
const { build } = support('esbuild');
const { values: { samples, cpu }, positionals } = parseArgs({
  allowPositionals: true,
  options: {
    samples: { type: 'string', default: '30' },
    cpu: { type: 'string', default: '1' },
  },
});
assert(positionals.length <= 1, 'Usage: pnpm bench:conditions [baseline-ref] [--samples 30] [--cpu 1]');
const count = Number(samples);
const slowdown = Number(cpu);
assert(Number.isInteger(count) && count > 0 && Number.isFinite(slowdown) && slowdown >= 1);
const git = (...args) => execFileSync('git', ['--no-pager', ...args], { cwd: root, encoding: 'utf8' });
const baseline = positionals[0] ? git('rev-parse', '--verify', '--end-of-options', `${positionals[0]}^{tree}`).trim() : null;
const iterations = { direct: 6_000_000, plan: 6_000_000, steady: 512, toggle: 512 };

async function bundle(reference) {
  const { outputFiles } = await build({
    entryPoints: [resolve(pkg, 'benchmarks', 'conditions-entry.ts')],
    tsconfig: resolve(pkg, 'tsconfig.test.json'),
    bundle: true, minify: true, write: false, format: 'esm', target: 'es2022',
    supported: { 'import-attributes': true },
    define: { __WEBUI_DEV__: 'false' },
    plugins: reference ? [{
      name: 'condition-benchmark-baseline',
      setup(plugin) {
        plugin.onLoad({ filter: /[\\/]webui-framework[\\/]src[\\/].*\.ts$/ }, ({ path }) => ({
          contents: git('show', `${reference}:${relative(root, path).replaceAll('\\', '/')}`),
          loader: 'ts',
        }));
      },
    }] : [],
  });
  return outputFiles[0].text;
}

const bundles = {};
if (baseline) bundles.baseline = await bundle(baseline);
bundles.current = await bundle(null);
const browser = await chromium.launch();
const results = [];
try {
  for (const name of ['truthy', 'negated', 'comparison', 'compound']) {
    const pages = {};
    const errors = [];
    for (const [version, code] of Object.entries(bundles)) {
      const page = await browser.newPage();
      pages[version] = page;
      page.on('pageerror', error => errors.push(error.message));
      await page.route('http://conditions.test/**', route => {
        if (new URL(route.request().url()).pathname === '/bench.js') {
          return route.fulfill({ contentType: 'text/javascript', body: code });
        }
        return route.fulfill({
          contentType: 'text/html',
          headers: {
            'content-security-policy': "script-src 'nonce-condition-bench'; require-trusted-types-for 'script'; trusted-types webui",
          },
          body: '<!doctype html><body><script type="module" nonce="condition-bench" src="/bench.js"></script></body>',
        });
      });
      if (slowdown !== 1) {
        const cdp = await page.context().newCDPSession(page);
        await cdp.send('Emulation.setCPUThrottlingRate', { rate: slowdown });
      }
      await page.goto(`http://conditions.test/?case=${name}`);
      assert.deepEqual(errors, []);
      await page.waitForFunction(() => typeof window.conditionBench === 'function');
    }
    const cases = [
      { version: 'current', kind: 'direct', unit: 'ns/check', samples: [] },
      ...Object.keys(pages).flatMap(version => [
        { version, kind: 'plan', unit: 'ns/check', samples: [] },
        { version, kind: 'steady', unit: 'us/update', samples: [] },
        { version, kind: 'toggle', unit: 'us/update', samples: [] },
      ]),
    ];
    for (let sample = -6; sample < count; sample++) {
      const order = sample % 2 === 0 ? cases : cases.slice().reverse();
      for (const row of order) {
        const repeats = iterations[row.kind];
        const elapsed = await pages[row.version].evaluate(({ kind, repeats }) =>
          window.conditionBench(kind, repeats), { kind: row.kind, repeats });
        if (sample >= 0) row.samples.push(elapsed);
      }
    }
    assert.deepEqual(errors, []);
    for (const page of Object.values(pages)) await page.close();
    for (const row of cases) {
      const sorted = row.samples.slice().sort((a, b) => a - b);
      results.push({
        name, ...row,
        p50: (sorted[Math.floor((count - 1) / 2)] + sorted[Math.floor(count / 2)]) / 2,
        p95: sorted[Math.ceil(.95 * count) - 1],
      });
    }
  }
  const bundleHashes = Object.fromEntries(Object.entries(bundles).map(([version, code]) =>
    [version, createHash('sha256').update(code).digest('hex')]));
  console.log(JSON.stringify({ baseline, head: git('rev-parse', 'HEAD').trim(), bundleHashes, iterations,
    cpuSlowdown: slowdown, chromium: browser.version(), node: process.version, results }, null, 2));
} finally {
  await browser.close();
}
