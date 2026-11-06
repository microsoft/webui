// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { execFileSync } from 'node:child_process';
import { createRequire } from 'node:module';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { gzipSync } from 'node:zlib';
import { minifyClient } from '../../../examples/minify-client.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..', '..');
const require = createRequire(join(root, 'packages', 'webui-framework', 'package.json'));
const support = createRequire(join(root, 'packages', 'webui-test-support', 'package.json'));
const { build } = support('esbuild');
const { chromium } = require('@playwright/test');
const [baseline, comparison] = process.argv.slice(2);
if (!baseline) {
  throw new Error('Usage: node benchmarks/render-completion.mjs <baseline-ref> [comparison-ref]. Start tests/server.ts first.');
}
const references = { baseline, ...(comparison ? { comparison } : {}), current: undefined };
const binding = process.env.BENCH_BINDING ?? 'attribute';
if (binding !== 'attribute' && binding !== 'complex') {
  throw new Error('BENCH_BINDING must be attribute or complex.');
}
const collection = binding === 'complex' ? 'complexRows' : 'rows';
const output = { samples: 30, batchesPerSample: 20, binding, references, bundles: {}, cases: [] };
const percentile = (values, p) => [...values].sort((a, b) => a - b)[Math.ceil(values.length * p) - 1];
const git = args => execFileSync('git', args, { cwd: root, encoding: 'utf8' });
const browser = await chromium.launch({ args: ['--js-flags=--expose-gc', '--enable-precise-memory-info'] });
output.environment = { node: process.version, chromium: browser.version() };
try {
  const pages = new Map();
  for (const [version, reference] of Object.entries(references)) {
    const sources = new Map();
    if (reference) {
      for (const path of git(['ls-tree', '-r', '--name-only', reference, '--', 'packages/webui-framework/src']).trim().split('\n')) {
        if (path.endsWith('.ts')) sources.set(path, git(['show', `${reference}:${path}`]));
      }
    }
    const result = await build({
      entryPoints: [join(root, 'packages', 'webui-framework', 'tests', 'fixtures', 'render-completion', 'element.ts')],
      bundle: true, minify: false, format: 'iife', platform: 'browser', target: 'es2022', write: false,
      supported: { 'import-attributes': true },
      metafile: true, outfile: join(root, 'target', `render-benchmark-${version}.js`),
      define: { __WEBUI_DEV__: 'false' },
      tsconfig: join(root, 'packages', 'webui-framework', 'tsconfig.test.json'),
      plugins: [minifyClient(), {
        name: 'snapshot-source',
        setup(plugin) {
          if (!reference) return;
          plugin.onLoad({ filter: /[\\/]webui-framework[\\/]src[\\/].*\.ts$/ }, ({ path }) => {
            const key = relative(root, path).replaceAll('\\', '/');
            const contents = sources.get(key);
            if (contents === undefined) throw new Error(`Missing ${key} in snapshot ${reference}`);
            return { contents, loader: 'ts' };
          });
        },
      }],
    });
    const code = result.outputFiles[0].text;
    output.bundles[version] = { bytes: Buffer.byteLength(code), gzip: gzipSync(code).length };
    const page = await browser.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.route('**/dist/render-completion/element.js', route => route.fulfill({
      contentType: 'application/javascript', body: code,
    }));
    await page.addInitScript(() => {
      Object.defineProperty(window, 'IntersectionObserver', {
        value: class { observe() {} unobserve() {} disconnect() {} },
      });
    });
    await page.goto(`http://127.0.0.1:${process.env.PORT ?? 39101}/render-completion/fixture.html`);
    await page.waitForFunction(() => document.querySelector('test-completion-root')?.$ready);
    const completion = await page.evaluate(() => typeof document.querySelector('test-completion-root').$whenRendered === 'function');
    pages.set(version, { page, errors, completion });
  }
  for (const count of [10, 1000]) {
    for (const { page } of pages.values()) {
      await page.evaluate(async ({ count, collection }) => {
        const flush = element => (element.$flushUpdates ?? element[Symbol.for('microsoft.webui.flushUpdates')]).call(element);
        const root = document.querySelector('test-completion-root');
        root.rows = [];
        root.complexRows = [];
        root[collection] = Array.from({ length: count }, (_, id) => ({ id: String(id), value: '1' }));
        flush(root);
        await new Promise(resolve => setTimeout(resolve, 0));
        window.benchChildren = [...root.shadowRoot.querySelectorAll('test-completion-child')];
        window.benchLeaves = window.benchChildren.map(child => child.shadowRoot.querySelector('test-completion-leaf'));
        window.benchIteration = 0;
        if (window.benchChildren.length !== count + 1) throw new Error('Row setup mismatch');
      }, { count, collection });
    }
    for (const sparse of [false, true]) {
      const results = [];
      for (const [version, { completion }] of pages) {
        const modes = ['automatic'];
        if (version !== 'current') modes.push('manual-cached', 'manual-query');
        if (completion) modes.push('await');
        for (const mode of modes) results.push({ version, mode, count, sparse, times: [] });
      }
      for (let sample = -5; sample < output.samples; sample++) {
        for (let offset = 0; offset < results.length; offset++) {
          const result = results[(sample + 5 + offset) % results.length];
          const elapsed = await pages.get(result.version).page.evaluate(async ({ count, sparse, mode, batches, collection }) => {
            const flush = element => (element.$flushUpdates ?? element[Symbol.for('microsoft.webui.flushUpdates')]).call(element);
            const root = document.querySelector('test-completion-root');
            const before = performance.now();
            for (let batch = 0; batch < batches; batch++) {
              const value = String(++window.benchIteration);
              if (sparse) root.value = value;
              else root[collection] = Array.from({ length: count }, (_, id) => ({ id: String(id), value }));
              if (mode === 'await') await root.$whenRendered();
              else if (mode === 'automatic') {
                // This fixture has exactly three update levels. Equal turns in
                // every snapshot isolate normal scheduling without observers.
                await Promise.resolve();
                await Promise.resolve();
                await Promise.resolve();
              } else {
                flush(root);
                const children = mode === 'manual-query'
                  ? [...root.shadowRoot.querySelectorAll('test-completion-child')]
                  : window.benchChildren;
                for (const child of children) flush(child);
                const leaves = mode === 'manual-query'
                  ? children.map(child => child.shadowRoot.querySelector('test-completion-leaf'))
                  : window.benchLeaves;
                for (const leaf of leaves) flush(leaf);
                await Promise.resolve();
              }
              const index = sparse ? 0 : count;
              if (window.benchLeaves[index].querySelector('.action').textContent !== String(Number(value) + 1)) {
                throw new Error('Rendering returned before nested DOM completed');
              }
            }
            const elapsed = performance.now() - before;
            const children = [...root.shadowRoot.querySelectorAll('test-completion-child')];
            if (children.length !== count + 1) throw new Error('Rendered row count changed');
            const expected = String(Number(window.benchIteration) + 1);
            for (let i = sparse ? 0 : 1; i < (sparse ? 1 : children.length); i++) {
              const leaf = children[i].shadowRoot.querySelector('test-completion-leaf');
              if (leaf.querySelector('.action').textContent !== expected) {
                throw new Error(`Incorrect nested DOM at row ${i}`);
              }
              if (!sparse && collection === 'complexRows' &&
                  children[i].shadowRoot.querySelector('.label').textContent !== String(window.benchIteration)) {
                throw new Error(`Incorrect complex binding at row ${i}`);
              }
            }
            return elapsed;
          }, { count, sparse, mode: result.mode, batches: output.batchesPerSample, collection });
          if (sample >= 0) result.times.push(elapsed);
        }
      }
      for (const result of results) {
        result.p50 = percentile(result.times, .5);
        result.p95 = percentile(result.times, .95);
        output.cases.push(result);
        console.error(JSON.stringify({ ...result, times: undefined }));
      }
    }
  }
  output.memory = {};
  output.retainedTree = {};
  output.retainedHeap = {};
  for (const [version, { page, errors, completion }] of pages) {
    if (errors.length) throw new Error(`${version}: ${errors.join('; ')}`);
    const session = await page.context().newCDPSession(page);
    await session.send('HeapProfiler.collectGarbage');
    const heap = await session.send('Runtime.getHeapUsage');
    output.retainedTree[version] = heap.usedSize;
    output.retainedHeap[version] = { ...heap, snapshot: await snapshotTotals(session) };
    const series = [];
    for (let round = 0; round < 3; round++) {
      await page.evaluate(async ({ completion, collection }) => {
        const flush = element => (element.$flushUpdates ?? element[Symbol.for('microsoft.webui.flushUpdates')]).call(element);
        const root = document.querySelector('test-completion-root');
        window.benchChildren = null;
        window.benchLeaves = null;
        for (let i = 0; i < 50; i++) {
          root[collection] = Array.from({ length: 100 }, (_, id) => ({ id: `${i}-${id}`, value: String(i) }));
          if (completion) await root.$whenRendered();
          else {
            flush(root);
            await new Promise(resolve => setTimeout(resolve, 0));
          }
        }
        root[collection] = [];
        flush(root);
        await new Promise(resolve => setTimeout(resolve, 0));
      }, { completion, collection });
      await session.send('HeapProfiler.collectGarbage');
      series.push((await session.send('Runtime.getHeapUsage')).usedSize);
    }
    output.memory[version] = series;
    if (errors.length) throw new Error(`${version}: ${errors.join('; ')}`);
  }
  console.log(JSON.stringify(output, null, 2));
} finally {
  await browser.close();
}

async function snapshotTotals(session) {
  const chunks = [];
  const receive = event => chunks.push(event.chunk);
  session.on('HeapProfiler.addHeapSnapshotChunk', receive);
  try {
    await session.send('HeapProfiler.takeHeapSnapshot', { reportProgress: false });
  } finally {
    session.off('HeapProfiler.addHeapSnapshotChunk', receive);
  }
  const { snapshot: { meta }, nodes, strings } = JSON.parse(chunks.join(''));
  const width = meta.node_fields.length;
  const sizeIndex = meta.node_fields.indexOf('self_size');
  const typeIndex = meta.node_fields.indexOf('type');
  const nameIndex = meta.node_fields.indexOf('name');
  const types = meta.node_types[typeIndex];
  if (sizeIndex < 0 || typeIndex < 0 || nameIndex < 0 || !Array.isArray(types)) {
    throw new Error('Unsupported Chrome heap snapshot metadata.');
  }
  const totals = { total: 0, externalStrings: 0, byType: {} };
  for (let index = 0; index < nodes.length; index += width) {
    const size = nodes[index + sizeIndex];
    const type = types[nodes[index + typeIndex]];
    totals.total += size;
    totals.byType[type] = (totals.byType[type] ?? 0) + size;
    if (type === 'native' && strings[nodes[index + nameIndex]] === 'system / ExternalStringData') {
      totals.externalStrings += size;
    }
  }
  return totals;
}
