// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { once } from 'node:events';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import { basename, dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { brotliCompressSync, gzipSync } from 'node:zlib';
import { chromium } from '@playwright/test';
import { build as bundle } from 'esbuild';
import { build } from '../../webui/dist/build.js';
import { Protocol } from '../../webui/dist/index.js';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '..', '..', '..');
const { values } = parseArgs({ options: {
  samples: { type: 'string', default: '24' },
  'heap-samples': { type: 'string', default: '8' },
  cpu: { type: 'string', default: '4' },
  latency: { type: 'string', default: '80' },
} });
const config = {
  samples: Number(values.samples),
  heapSamples: Number(values['heap-samples']),
  cpu: Number(values.cpu),
  latencyMs: Number(values.latency),
};
for (const [key, value] of Object.entries(config)) {
  assert(Number.isSafeInteger(value) && value >= (key === 'latencyMs' ? 0 : 1), `Invalid ${key}`);
}

const nonce = 'condition-route-benchmark';
const policy = `default-src 'self'; script-src 'nonce-${nonce}' 'strict-dynamic'; style-src 'self' 'nonce-${nonce}'; style-src-attr 'unsafe-inline'; require-trusted-types-for 'script'; trusted-types webui`;
const boot = '/condition-routes-entry.js';
const routeModule = '/condition-routes-page.js';
const ids = ['plans', 'direct'];
const result = { config, node: process.version, cases: [] };
const folder = await mkdtemp(join(tmpdir(), 'webui-condition-routes-'));
const server = createServer();
let browser;
try {
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  const origin = `http://127.0.0.1:${server.address().port}`;
  browser = await chromium.launch();
  result.chromium = browser.version();
  let active;
  server.on('request', (req, res) => {
    const path = new URL(req.url, origin).pathname;
    const asset = active?.get(path);
    if (!asset) {
      res.writeHead(path === '/favicon.ico' ? 204 : 404).end();
      return;
    }
    res.writeHead(200, {
      'cache-control': 'no-store',
      'content-type': asset.type,
      'content-encoding': 'gzip',
      'content-length': asset.gzip.length,
      ...(path === '/' ? { 'content-security-policy': policy } : {}),
    }).end(asset.gzip);
  });
  for (const status of [false, true]) {
    const protocol = await compileFixture(status);
    const startup = protocol.render({ $webui: {
      headEnd: `<meta name="webui-nonce" content="${nonce}">`,
      bodyEnd: `<script type="module" nonce="${nonce}" src="${boot}"></script>`,
    } }, { entry: 'index.html', requestPath: '/' });
    const built = await Promise.all(ids.map(id => clientAssets(id, status)));
    assert.deepEqual(built[0].startup, built[1].startup, 'Only cold route assets may differ');
    for (const rows of [1, 1_000]) {
      const state = { rows: Array.from({ length: rows }, () => ({ count: 0, status: 'idle' })) };
      const raw = protocol.renderPartial(state, 'index.html', '/comparisons', '');
      const partial = JSON.parse(raw);
      const condition = partial.templates['bench-comparison-row'].c[0][0];
      assert.deepEqual(condition, status ? [[3, 0, 'active'], ['status']] : [[1, 0, 0], ['count']]);
      condition[0] = null;

      const cases = built.map(({ files, startup: startupBytes }, i) => {
        const assets = new Map(files);
        assets.set('/', compress(startup, 'text/html'));
        assets.set('/comparisons', compress(i ? JSON.stringify(partial) : raw, 'application/json'));
        const coldGzip = assets.get('/comparisons').gzip.length + files.get(routeModule).gzip.length;
        return {
          assets,
          output: { variant: ids[i], expression: status ? 'status' : 'count', rows, startupBytes, coldGzip,
            cold: [], warm: {}, retained: [] },
        };
      });
      for (const kind of ['timing', 'heap']) {
        const count = kind === 'timing' ? config.samples : config.heapSamples;
        for (let i = kind === 'timing' ? -2 : 0; i < count; i++) {
          for (const trial of i % 2 === 0 ? cases : [...cases].reverse()) {
            active = trial.assets;
            await sample(origin, browser, trial.output, kind, i >= 0);
          }
        }
      }
      for (const { output } of cases) {
        output.cold = summarize(output.cold);
        for (const key of Object.keys(output.warm)) output.warm[key] = summarize(output.warm[key]);
        output.retained = Object.fromEntries(['activeJs', 'leftJs', 'activeEmbedder', 'leftEmbedder']
          .map(key => [key, summarize(output.retained.map(sample => sample[key]))]));
        result.cases.push(output);
      }
    }
  }
  console.log(JSON.stringify(result, null, 2));
} finally {
  try {
    await browser?.close();
    if (server.listening) await new Promise(resolve => server.close(resolve));
  } finally {
    await rm(folder, { recursive: true, force: true });
  }
}

async function compileFixture(status) {
  const files = {
    'index.html': `<!doctype html><html><head><title>Conditions</title></head><body>
      <route path="/" component="bench-route-shell">
        <route path="" component="bench-route-home" exact />
        <route path="comparisons" component="bench-comparison-page" exact />
      </route></body></html>`,
    'bench-route-shell/bench-route-shell.html': `<nav><a href="/" id="close-comparisons">Home</a>
      <a href="/comparisons" id="open-comparisons">Comparisons</a></nav><main><outlet /></main>`,
    'bench-route-home/bench-route-home.html': '<h1>Home</h1>',
    'bench-comparison-page/bench-comparison-page.html': `<for each="row in rows">
      <bench-comparison-row :${status ? 'status="{{row.status}}"' : 'count="{{row.count}}"'}></bench-comparison-row>
    </for>`,
    'bench-comparison-row/bench-comparison-row.html': `<if condition="${status ? "status == 'active'" : 'count > 0'}">
      <span class="matched">Matched</span></if>`,
    'bench-comparison-page/bench-comparison-page.ts': '',
    'bench-comparison-row/bench-comparison-row.ts': '',
  };
  for (const [file, contents] of Object.entries(files)) {
    const path = join(folder, file);
    await mkdir(dirname(path), { recursive: true });
    await writeFile(path, contents);
  }
  const compiled = build({ appDir: folder, plugin: 'webui', dom: 'light', css: 'style' });
  return new Protocol(compiled.protocol, { plugin: 'webui' });
}

function compress(data, type) {
  return { type, gzip: gzipSync(data) };
}

async function clientAssets(id, status) {
  const outdir = join(here, 'out');
  const built = await bundle({
    absWorkingDir: root, format: 'esm', splitting: true, bundle: true, minify: true, write: false, metafile: true,
    entryPoints: [join(here, basename(boot, '.js') + '.ts'), join(here, basename(routeModule, '.js') + '.ts')],
    outdir, tsconfig: join(root, 'tsconfig.json'), target: 'es2022', supported: { 'import-attributes': true },
    define: { __WEBUI_DEV__: 'false', __BENCH_DIRECT__: String(id === 'direct'), __BENCH_STATUS__: String(status) },
  });
  const url = path => `/${relative(outdir, path).split('\\').join('/')}`;
  const files = new Map(built.outputFiles.map(file => [url(file.path), compress(file.contents, 'text/javascript')]));
  const bytes = new Map(built.outputFiles.map(file => [url(file.path), file.contents]));
  const output = Object.entries(built.metafile.outputs).map(([path, info]) => [url(resolve(root, path)), info]);
  const startupFiles = new Set([boot]);
  const metadata = new Map(output);
  for (const key of startupFiles) {
    for (const dep of metadata.get(key).imports) {
      if (!dep.external && dep.kind !== 'dynamic-import') startupFiles.add(url(resolve(root, dep.path)));
    }
  }
  assert(!startupFiles.has(routeModule), 'The route module must not be on the startup path');
  const startup = { requests: startupFiles.size, raw: 0, gzip: 0, brotli: 0, sha256: '' };
  const hash = createHash('sha256');
  for (const path of startupFiles) {
    const data = bytes.get(path);
    hash.update(data);
    startup.raw += data.length;
    startup.gzip += files.get(path).gzip.length;
    startup.brotli += brotliCompressSync(data).length;
  }
  startup.sha256 = hash.digest('hex');
  return { files, startup };
}

async function sample(origin, browser, output, kind, record) {
  const context = await browser.newContext();
  const errors = [];
  const requests = [];
  try {
    const page = await context.newPage();
    page.setDefaultTimeout(12_000);
    page.on('pageerror', error => errors.push(error.message));
    page.on('console', message => {
      if (message.type() === 'error' || message.type() === 'warning') errors.push(message.text());
    });
    page.on('request', request => requests.push(new URL(request.url()).pathname));
    await page.addInitScript(() => {
      window.conditionViolations = [];
      document.addEventListener('securitypolicyviolation', event => window.conditionViolations.push(event.effectiveDirective));
    });
    const cdp = await context.newCDPSession(page);
    await cdp.send('Emulation.setCPUThrottlingRate', { rate: config.cpu });
    if (config.latencyMs !== 0) {
      await cdp.send('Network.emulateNetworkConditions', {
        offline: false, latency: config.latencyMs, downloadThroughput: 200_000, uploadThroughput: 93_750,
      });
    }
    const response = await page.goto(origin);
    assert.equal(await response.headerValue('content-security-policy'), policy);
    await page.waitForFunction(() => window.conditionRouteBenchmark?.active() === 'bench-route-home');
    assert.equal(await page.evaluate(() => Boolean(window.__webui.templates['bench-comparison-row'])), false);
    const before = kind === 'heap' ? await readHeap(page, cdp) : undefined;
    const start = requests.length;
    await page.locator('#open-comparisons').click();
    const cold = await (await page.waitForFunction(() => window.conditionRouteBenchmark?.cold)).jsonValue();
    assert.equal(cold.rows, output.rows);
    assert.deepEqual(requests.slice(start).sort(), ['/comparisons', routeModule].sort(), 'Cold route request count changed');
    if (record && kind === 'timing') {
      output.cold.push(cold.elapsed);
      for (const scope of output.rows === 1 ? ['one'] : ['one', 'all']) {
        for (const mode of ['flip', 'stable']) {
          const time = await page.evaluate(measureUpdate, { expression: output.expression, scope, mode });
          (output.warm[`${scope}/${mode}`] ??= []).push(time);
        }
      }
    } else if (kind === 'heap') {
      const active = await readHeap(page, cdp);
      await page.locator('#close-comparisons').click();
      await page.waitForFunction(() => window.conditionRouteBenchmark?.active() === 'bench-route-home'
        && !document.querySelector('bench-comparison-page'));
      const left = await readHeap(page, cdp);
      output.retained.push({
        activeJs: active.usedSize - before.usedSize, leftJs: left.usedSize - before.usedSize,
        activeEmbedder: active.embedderHeapUsedSize - before.embedderHeapUsedSize,
        leftEmbedder: left.embedderHeapUsedSize - before.embedderHeapUsedSize,
      });
    }
    assert.deepEqual(errors, []);
    assert.deepEqual(await page.evaluate(() => window.conditionViolations), []);
  } catch (cause) {
    throw new Error(`${output.variant}/${output.expression}/${output.rows} (${kind}): ${requests.join(', ')}\n${errors.join('\n')}`, { cause });
  } finally {
    await context.close();
  }
}

async function measureUpdate({ expression, scope, mode }) {
  const host = document.querySelector('bench-comparison-page');
  const rows = host.querySelectorAll('bench-comparison-row');
  const count = scope === 'one' ? 1 : rows.length;
  const untouched = count < rows.length ? rows[count].querySelector('.matched') : null;
  const status = expression === 'status';
  const flip = mode === 'flip';
  const [first, second] = status ? (flip ? ['idle', 'active'] : ['idle', 'pending']) : (flip ? [0, 1] : [1, 2]);
  const batches = count === 1 ? 64 : 12;
  const update = value => {
    for (let i = 0; i < count; i++) rows[i][expression] = value;
    return host.$whenRendered();
  };
  const verify = matched => {
    for (let i = 0; i < count; i++) {
      if (Boolean(rows[i].querySelector('.matched')) !== matched) throw new Error(`Incorrect ${mode} DOM for row ${i}`);
    }
  };
  await update(second);
  verify(flip || !status);
  await update(first);
  verify(!flip && !status);
  const start = performance.now();
  for (let i = 0; i < batches; i++) await update(i % 2 === 0 ? second : first);
  const elapsed = performance.now() - start;
  verify(!flip && !status);
  if (count < rows.length && untouched !== rows[count].querySelector('.matched')) {
    throw new Error('A one-row update changed an untouched row.');
  }
  return elapsed / batches;
}

async function readHeap(page, cdp) {
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  await cdp.send('HeapProfiler.collectGarbage');
  return cdp.send('Runtime.getHeapUsage');
}

function summarize(samples) {
  const sorted = [...samples].sort((a, b) => a - b);
  return { p50: sorted[Math.ceil(sorted.length * .5) - 1], p95: sorted[Math.ceil(sorted.length * .95) - 1], samples };
}
