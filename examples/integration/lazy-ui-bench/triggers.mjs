// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// Pay-for-play evidence for `<if lazy>` prefetch triggers, using the real
// compiler, framework runtime, and build-time template packs.
//
// Each variant is the same app compiled with a different body declaration:
// plain `<if>` or `<if lazy="demand|intent|idle|eager">`. Lazy variants import
// the generated `lazy.webui.js`, as applications do. The driver measures
// startup bytes, JS heap, and blocking time, click-to-paint for one dialog,
// a second open, and heap after closed regions are released.
//
//   node examples/integration/lazy-ui-bench/triggers.mjs [--samples 20] [--profile mobile|none]
//        [--scenarios s1,s30] [--variants plain,demand,intent,idle,eager] [--caches cold,warm]
//        [--memory-samples 5] [--out target/lazy-triggers.json]

import assert from 'node:assert/strict';
import { once } from 'node:events';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import { cpus, platform, release } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { brotliCompressSync, constants, gzipSync } from 'node:zlib';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..', '..');
const require = createRequire(join(root, 'packages', 'webui-test-support', 'package.json'));
const { build: bundle, version: esbuildVersion } = require('esbuild');
const { chromium } = require('@playwright/test');
assert.ok(process.env.WEBUI_ADDON_PATH, 'Set WEBUI_ADDON_PATH to the freshly built release addon.');
const { build, Protocol } = await import(pathToFileURL(join(root, 'packages', 'webui', 'dist', 'index.js')).href);
const { esbuildProjection } = await import(pathToFileURL(join(root, 'packages', 'webui', 'dist', 'projection', 'index.js')).href);

const args = Object.fromEntries(process.argv.slice(2).reduce((pairs, value, index, all) => {
  if (value.startsWith('--')) pairs.push([value.slice(2), all[index + 1]]);
  return pairs;
}, []));
const list = (value, fallback) => (value ?? fallback).split(',');
const SAMPLES = Number(args.samples ?? 20);
const MEMORY_SAMPLES = Number(args['memory-samples'] ?? 5);
const WARMUPS = 1;
const DWELL_MS = 200;
const PRIMER_TEARDOWN_MS = 1_500;
const PROFILES = {
  // Lighthouse mobile: 150 ms RTT, 1.6 Mbps down, 750 Kbps up, 4x CPU.
  mobile: { latency: 150, down: 1_600_000 / 8, up: 750_000 / 8, cpu: 4 },
  none: undefined,
};
const PROFILE_NAME = args.profile ?? 'mobile';
const PROFILE = PROFILES[PROFILE_NAME];
const SCENARIOS = {
  s1: { dialogs: 1, fields: 24, rows: 5, target: 0 },
  s30: { dialogs: 30, fields: 24, rows: 5, target: 17 },
};
const scenarios = list(args.scenarios, 's1,s30');
const variants = list(args.variants, 'plain,demand,intent,idle,eager');
const caches = list(args.caches, 'cold,warm');
const OUTPUT = resolve(args.out ?? join(root, 'target', 'lazy-triggers.json'));
// Package specifiers let the projection plugin recognize the framework's decorators.
const FRAMEWORK = {
  'index.ts': '@microsoft/webui-framework',
  'runtime.ts': '@microsoft/webui-framework/runtime.js',
  'styles-entry.ts': '@microsoft/webui-framework/styles.js',
  'template-cache-entry.ts': '@microsoft/webui-framework/template-cache.js',
};
const framework = file => JSON.stringify(FRAMEWORK[file]);

const WORDS = ('account access alerts archive backup billing calendar channel comments contacts content currency '
  + 'density desktop devices display drafts editor email export filters folders fonts history identity images '
  + 'imports invoices keyboard language layout library location messages mobile network notices offline orders '
  + 'payment people photos privacy profile projects quality region reminders reports schedule search security '
  + 'sessions sharing shipping signals storage streams summary sync teams themes timeline updates uploads usage widgets')
  .split(' ');
const word = (n, f, salt) => WORDS[(n * 31 + f * 7 + salt * 13) % WORDS.length];
const title = text => text[0].toUpperCase() + text.slice(1);
const ownerTag = n => `bench-dialog-${String(n).padStart(2, '0')}`;

function dialogBody(n, fields) {
  let html = `<header class="sheet-head"><h2>${title(word(n, 0, 1))} settings ${n}</h2>`
    + '<bench-badge label="{{badge}}"></bench-badge>'
    + '<button type="button" class="close" @click="{close()}" aria-label="Close">&#215;</button></header>'
    + '<p class="intro">{{intro}}</p><form class="sheet-form" @submit="{apply(e)}">';
  for (let f = 0; f < fields; f++) {
    if (f % 8 === 0) html += `${f > 0 ? '</fieldset>' : ''}<fieldset><legend>${title(word(n, f, 2))} and ${word(n, f, 3)}</legend>`;
    const label = `${title(word(n, f, 4))} ${word(n, f, 5)}`;
    html += [
      `<label class="field">${label}<input name="f${f}" value="{{text}}"></label>`,
      `<label class="check"><input type="checkbox" name="f${f}" ?checked="{{enabled}}">${label}</label>`,
      `<label class="field">${label}<select name="f${f}" value="{{choice}}"><for each="option in options"><option value="{{option.value}}">{{option.label}}</option></for></select></label>`,
      `<label class="field">${label}<textarea name="f${f}" rows="2">Notes about ${word(n, f, 6)}.</textarea></label>`,
    ][(n + f) % 4];
  }
  return `${html}</fieldset><ul class="items"><for each="item in items"><li><strong>{{item.name}}</strong>`
    + '<span>{{item.detail}}</span></li></for></ul><footer class="actions"><button type="button" @click="{close()}">Cancel</button>'
    + '<button type="submit" class="primary">Apply</button></footer></form>';
}

const OWNER_CSS = `:host { display: block; }
button { font: inherit; border: 1px solid #c8ccd2; border-radius: 6px; background: #fff; padding: 6px 12px; }
.sheet { width: min(720px, 92vw); max-height: 86vh; border: 0; border-radius: 12px; padding: 0; }
.sheet::backdrop { background: rgb(15 20 25 / 0.45); }
.sheet-head { display: flex; align-items: center; gap: 12px; padding: 16px 20px; border-bottom: 1px solid #e3e6ea; }
.sheet-head h2 { margin: 0; font-size: 1.2rem; flex: 1; }
.intro { margin: 12px 20px; color: #59636e; }
.sheet-form { padding: 0 20px 20px; display: grid; gap: 12px; }
fieldset { border: 1px solid #e3e6ea; border-radius: 8px; display: grid; gap: 8px; grid-template-columns: repeat(auto-fill, minmax(200px, 1fr)); }
.field { display: grid; gap: 4px; font-size: 0.9rem; }
.check { display: flex; gap: 8px; align-items: center; font-size: 0.9rem; }
input, select, textarea { font: inherit; padding: 6px 8px; border: 1px solid #c8ccd2; border-radius: 6px; }
.items { margin: 0; padding: 0; list-style: none; display: grid; gap: 4px; }
.items li { display: flex; justify-content: space-between; padding: 6px 8px; background: #f6f7f9; border-radius: 6px; }
.actions { display: flex; justify-content: flex-end; gap: 8px; }
`;

// In-page harness shared by every variant: only the compiled body declaration differs.
const CLIENT = `
const longTasks: Array<[number, number]> = [];
try {
  new PerformanceObserver(list => {
    for (const entry of list.getEntries()) longTasks.push([entry.startTime, entry.duration]);
  }).observe({ type: 'longtask', buffered: true });
} catch {}
let lastPointer = 0;
document.addEventListener('pointerdown', event => { lastPointer = event.timeStamp; }, true);
const regions = new Map<string, any>();
const paint = () => new Promise<void>(resolve => requestAnimationFrame(() => setTimeout(resolve, 0)));
export const bench = {
  owners: [] as string[], target: '', hydratedAt: 0, appHydrated: false,
  result: undefined as Record<string, number> | undefined, error: '', longTasks,
  get hydrated(): boolean { return this.appHydrated && regions.size === this.owners.length; },
  configure(owners: string[], target: string) { this.owners = owners; this.target = target; },
  app() { this.appHydrated = true; this.mark(); },
  region(element: any) { regions.set(element.localName, element); this.mark(); },
  mark() { if (this.hydrated && !this.hydratedAt) this.hydratedAt = performance.now(); },
  loaded(): number {
    const templates = (window as any).__webui?.templates ?? {};
    return this.owners.filter(owner => templates[owner + '::lazy:@0']).length;
  },
  async open(element: any): Promise<void> {
    this.result = undefined;
    const inputAt = lastPointer;
    const start = performance.now();
    try {
      element.ready = true;
      // Plain pages do not import lazy.js, which adds $rendered(); their body renders on flush.
      await (element.$rendered?.() ?? element.$flushUpdates());
      const rendered = performance.now();
      if (!element.dialog.open) element.dialog.showModal();
      const mounted = performance.now();
      await paint();
      const pack = performance.getEntriesByType('resource').find(entry =>
        entry.name.includes('/_webui/packs/' + element.localName + '.') && entry.startTime >= start - 1) as PerformanceResourceTiming | undefined;
      this.result = {
        inputAt: inputAt > 0 && start - inputAt < 2000 ? inputAt : start, start, rendered, mounted,
        painted: performance.now(), waitMs: pack ? Math.max(0, pack.responseEnd - start) : 0,
      };
    } catch (error) {
      this.error = String(error);
    }
  },
  close(element: any): void {
    element.dialog.close();
    element.ready = false;
    element.$flushUpdates();
  },
  async reopen(): Promise<Record<string, number> | undefined> {
    const element = regions.get(this.target);
    this.close(element);
    await paint();
    lastPointer = 0;
    await this.open(element);
    return this.result;
  },
  closeTarget(): void { this.close(regions.get(this.target)); },
  async cycle(): Promise<number> {
    for (const owner of this.owners) {
      await this.open(regions.get(owner));
      if (this.error) throw new Error(this.error);
      this.close(regions.get(owner));
    }
    return this.owners.length;
  },
};
(window as any).__bench = bench;
`;

function writeApp(dir, scenario, variant) {
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(dir, { recursive: true });
  const write = (name, text) => writeFileSync(join(dir, name), text);
  write('index.html', '<!DOCTYPE html><html lang="en"><head><meta charset="utf-8">'
    + '<meta name="viewport" content="width=device-width, initial-scale=1"><title>{{title}}</title>'
    + '<style>body { margin: 0; font: 16px/1.45 system-ui, sans-serif; background: #f6f7f9; }</style>'
    + '</head><body><bench-app></bench-app><script type="module" src="./app.js"></script></body></html>\n');
  let cards = '';
  for (let n = 0; n < scenario.dialogs; n++) {
    cards += `<section class="card"><h2>${title(word(n, 0, 1))} ${n}</h2><p>Review ${word(n, 1, 8)} preferences.</p>`
      + `<${ownerTag(n)}></${ownerTag(n)}></section>`;
  }
  write('bench-app.html', '<template shadowrootmode="open"><header><h1>{{title}}</h1>'
    + `<bench-badge label="{{status}}"></bench-badge></header><main class="grid">${cards}</main></template>\n`);
  write('bench-app.css', ':host { display: block; } .grid { display: grid; gap: 16px; padding: 24px; '
    + 'grid-template-columns: repeat(auto-fill, minmax(260px, 1fr)); } .card { background: #fff; padding: 16px; }\n');
  write('bench-badge.html', '<template shadowrootmode="open"><span class="badge">{{label}}</span></template>\n');
  write('bench-badge.css', '.badge { font-size: 0.75rem; padding: 2px 8px; border-radius: 999px; background: #e7f0ff; }\n');
  const lazy = variant === 'plain' ? '' : ` lazy="${variant}"`;
  const header = `import { WebUIElement, attr, observable } from ${framework('index.ts')};\nimport { bench } from './bench_client.js';\n`;
  write('bench_client.ts', CLIENT);
  write('bench_data.ts', 'export const OPTIONS = [{ value: "a", label: "Automatic" }, { value: "b", label: "Balanced" }, '
    + '{ value: "c", label: "Compact" }];\n'
    + `export const ITEMS = Array.from({ length: ${scenario.rows} }, (_, i) => ({ name: 'Item ' + (i + 1), detail: 'Detail ' + (i + 1) }));\n`);
  write('bench-badge.ts', `${header}export class BenchBadge extends WebUIElement { @attr label = ''; }\nBenchBadge.define('bench-badge');\n`);
  write('bench-app.ts', `${header}export class BenchApp extends WebUIElement {
  @observable title = ''; @observable status = '';
  protected override hydratedCallback(): void { bench.app(); }
}
BenchApp.define('bench-app');\n`);
  let imports = '';
  const owners = [];
  for (let n = 0; n < scenario.dialogs; n++) {
    const tag = ownerTag(n);
    owners.push(tag);
    imports += `import './${tag}.js';\n`;
    write(`${tag}.html`, '<template shadowrootmode="open"><button class="trigger" type="button" @click="{open()}">'
      + `Open ${word(n, 0, 1)} settings</button><dialog class="sheet" w-ref="{dialog}">`
      + `<if condition="ready"${lazy}>${dialogBody(n, scenario.fields)}</if></dialog></template>\n`);
    write(`${tag}.css`, OWNER_CSS);
    write(`${tag}.ts`, `${header}import { ITEMS, OPTIONS } from './bench_data.js';
export class BenchDialog${n} extends WebUIElement {
  @observable ready = false; @observable intro = 'Introduction ${n}.'; @observable badge = 'Local';
  @observable text = 'Value'; @observable enabled = true; @observable choice = 'b';
  @observable options = OPTIONS; @observable items = ITEMS;
  dialog!: HTMLDialogElement;
  protected override hydratedCallback(): void { bench.region(this); }
  open(): void { void bench.open(this); }
  close(): void { bench.close(this); }
  apply(event: Event): void { event.preventDefault(); this.close(); }
}
BenchDialog${n}.define('${tag}');\n`);
  }
  write('main.ts', `import { startRuntime } from ${framework('runtime.ts')};
import ${framework('styles-entry.ts')};
import ${framework('template-cache-entry.ts')};
${variant === 'plain' ? '' : `import './.webui/lazy.webui.js';\n`}import { bench } from './bench_client.js';
import './bench-badge.js';
import './bench-app.js';
${imports}bench.configure(${JSON.stringify(owners)}, '${ownerTag(scenario.target)}');
startRuntime();
`);
  return owners;
}

const compress = buffer => ({
  raw: buffer,
  br: brotliCompressSync(buffer, { params: { [constants.BROTLI_PARAM_QUALITY]: 11 } }),
  gzip: gzipSync(buffer, { level: 9 }),
});

async function generate() {
  // Projection requires sources, packages, and outputs on one filesystem root.
  mkdirSync(join(root, 'target'), { recursive: true });
  const scratch = mkdtempSync(join(root, 'target', 'lazy-triggers-'));
  const fixtures = new Map();
  try {
    for (const id of scenarios) {
      const scenario = SCENARIOS[id];
      for (const variant of variants) {
        const dir = join(scratch, id, variant);
        const owners = writeApp(dir, scenario, variant);
        const options = { appDir: dir, plugin: 'webui', css: 'style', modulesOut: join(dir, '.webui') };
        // The generated loader never depends on the projection, so it is built
        // first and bundled like application code.
        const generated = variant === 'plain' ? undefined : build(options).lazyScriptsModule;
        if (generated) {
          mkdirSync(options.modulesOut, { recursive: true });
          writeFileSync(join(options.modulesOut, generated.fileName), generated.content);
          writeFileSync(join(options.modulesOut, generated.declaration.fileName), generated.declaration.content);
        }
        const manifest = join(dir, 'webui-projection.json');
        const outfile = join(dir, 'dist', 'app.js');
        await bundle({
          entryPoints: [join(dir, 'main.ts')], outfile, bundle: true, format: 'esm', minify: true,
          platform: 'browser', target: 'es2022', legalComments: 'none', absWorkingDir: dir,
          nodePaths: [join(root, 'examples', 'app', 'lazy-ui', 'node_modules')],
          tsconfigRaw: { compilerOptions: { experimentalDecorators: true, useDefineForClassFields: false } },
          supported: { 'import-attributes': true }, define: { __WEBUI_DEV__: 'false' },
          plugins: [esbuildProjection({ manifest })], logLevel: 'silent',
        }).catch(error => {
          const notes = (error.errors ?? []).flatMap(entry => entry.notes ?? []).map(note => note.text);
          throw new Error(`${error.message}\n${notes.join('\n')}`);
        });
        const built = build({ ...options, projectionManifests: [manifest] });
        assert.deepEqual(built.lazyScriptsModule, generated, 'lazy.webui.js is stable across builds');
        const packs = new Map();
        for (const pack of built.lazyPackFiles) {
          packs.set(pack.fileName, compress(Buffer.from(pack.content)));
        }
        assert.equal(packs.size, variant === 'plain' ? 0 : scenario.dialogs);
        fixtures.set(`${id}/${variant}`, {
          id, variant, owners, target: ownerTag(scenario.target),
          protocol: new Protocol(built.protocol, { plugin: 'webui' }),
          script: compress(readFileSync(outfile)), packs,
        });
      }
    }
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
  return fixtures;
}

/** Dynamic HTML is compressed per request; scripts and packs are immutable static files. */
async function serve(fixtures) {
  const server = createServer((request, response) => {
    const url = new URL(request.url ?? '/', 'http://127.0.0.1');
    const [id, variant, ...rest] = url.pathname.split('/').filter(Boolean);
    const fixture = fixtures.get(`${id}/${variant}`);
    const brotli = /\bbr\b/.test(String(request.headers['accept-encoding'] ?? ''));
    const send = (type, cache, body) => {
      response.writeHead(200, {
        'content-type': type, 'cache-control': cache, vary: 'accept-encoding',
        ...(brotli ? { 'content-encoding': 'br' } : {}),
      });
      response.end(brotli ? body.br : body.raw);
    };
    if (!fixture) return response.writeHead(404).end();
    if (rest.length === 0) {
      const html = Buffer.from(fixture.protocol.render({ title: 'Workspace', status: 'Ready' }));
      const body = { raw: html, br: brotliCompressSync(html, { params: { [constants.BROTLI_PARAM_QUALITY]: 5 } }) };
      return send('text/html; charset=utf-8', 'no-store', body);
    }
    const immutable = 'public, max-age=31536000, immutable';
    if (rest.join('/') === 'app.js') return send('text/javascript', immutable, fixture.script);
    const pack = rest[0] === '_webui' && rest[1] === 'packs' ? fixture.packs.get(rest[2]) : undefined;
    if (pack) return send('application/json', immutable, pack);
    response.writeHead(404).end();
  });
  server.keepAliveTimeout = 30_000;
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  return { server, origin: `http://127.0.0.1:${server.address().port}` };
}

const tbt = tasks => tasks.reduce((sum, [, duration]) => sum + Math.max(0, duration - 50), 0);
const within = (tasks, from, to) => tasks.filter(([start, duration]) => start + duration > from && start < to);
const waitFor = (page, expression, timeout = 60_000) => page.waitForFunction(expression, null, { polling: 50, timeout });

async function heap(cdp) {
  await cdp.send('HeapProfiler.collectGarbage');
  await cdp.send('HeapProfiler.collectGarbage');
  return (await cdp.send('Runtime.getHeapUsage')).usedSize;
}

function track(cdp, phase) {
  const requests = new Map();
  cdp.on('Network.requestWillBeSent', event => {
    requests.set(event.requestId, { url: event.request.url, phase: phase.current, bytes: 0, cached: false });
  });
  cdp.on('Network.requestServedFromCache', event => { const r = requests.get(event.requestId); if (r) r.cached = true; });
  cdp.on('Network.responseReceived', event => {
    const r = requests.get(event.requestId);
    if (r) r.cached ||= Boolean(event.response.fromDiskCache || event.response.fromPrefetchCache);
  });
  cdp.on('Network.loadingFinished', event => { const r = requests.get(event.requestId); if (r) r.bytes = event.encodedDataLength; });
  return requests;
}

function network(requests, phase) {
  const result = { requests: 0, cacheHits: 0, bytes: 0, packs: 0 };
  for (const request of requests.values()) {
    if (request.phase !== phase) continue;
    result.requests++;
    if (request.url.includes('/_webui/packs/')) result.packs++;
    if (request.cached) result.cacheHits++;
    else result.bytes += request.bytes;
  }
  return result;
}

async function prepare(page, url, variant) {
  await page.goto(url, { waitUntil: 'commit' });
  await waitFor(page, 'window.__bench?.hydrated === true || !!window.__bench?.error');
  // Idle and eager triggers count as part of startup until every pack is registered.
  if (variant === 'idle' || variant === 'eager') await waitFor(page, 'window.__bench.loaded() === window.__bench.owners.length');
  await page.waitForTimeout(300);
}

async function sample(browser, origin, fixture, cache) {
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  const url = `${origin}/${fixture.id}/${fixture.variant}/`;
  try {
    if (cache === 'warm') {
      const primer = await context.newPage();
      await prepare(primer, url, fixture.variant);
      await primer.locator(fixture.target).locator('button.trigger').click();
      await waitFor(primer, '!!window.__bench.result || !!window.__bench.error');
      await primer.close();
      // Chromium tears down a closed page's compositor about a second later, on
      // the renderer main thread that same-origin pages share; measuring then
      // adds that teardown (hundreds of ms under CPU throttling) to the open.
      await delay(PRIMER_TEARDOWN_MS);
    }
    const page = await context.newPage();
    const cdp = await context.newCDPSession(page);
    const phase = { current: 'startup' };
    const requests = track(cdp, phase);
    await cdp.send('Network.enable');
    if (PROFILE) {
      await cdp.send('Network.emulateNetworkConditions', {
        offline: false, latency: PROFILE.latency, downloadThroughput: PROFILE.down, uploadThroughput: PROFILE.up,
      });
      await cdp.send('Emulation.setCPUThrottlingRate', { rate: PROFILE.cpu });
    }
    await prepare(page, url, fixture.variant);
    const startup = await page.evaluate(() => ({ hydratedAt: window.__bench.hydratedAt, tasks: window.__bench.longTasks.slice() }));
    const heapStartup = await heap(cdp);
    phase.current = 'open';
    const button = page.locator(fixture.target).locator('button.trigger');
    if (fixture.variant === 'intent') {
      await button.hover();
      await page.waitForTimeout(DWELL_MS);
    }
    await button.click();
    await waitFor(page, '!!window.__bench.result || !!window.__bench.error');
    const open = await page.evaluate(() => ({ result: window.__bench.result, error: window.__bench.error, tasks: window.__bench.longTasks.slice() }));
    if (open.error) throw new Error(open.error);
    const heapOpen = await heap(cdp);
    const reopen = await page.evaluate(() => window.__bench.reopen());
    phase.current = 'close';
    await page.evaluate(() => window.__bench.closeTarget());
    // Let the runtime release the closed region at idle.
    await page.waitForTimeout(1_000);
    const heapClosed = await heap(cdp);
    const timing = open.result;
    return {
      openMs: timing.painted - timing.inputAt,
      inputDelayMs: timing.start - timing.inputAt,
      waitMs: timing.waitMs,
      renderMs: timing.rendered - timing.start - timing.waitMs,
      modalMs: timing.mounted - timing.rendered,
      paintMs: timing.painted - timing.mounted,
      reopenMs: reopen.painted - reopen.start,
      openTbtMs: tbt(within(open.tasks, timing.inputAt, timing.painted)),
      hydratedMs: startup.hydratedAt,
      startupTbtMs: tbt(within(startup.tasks, 0, startup.hydratedAt + 2_000)),
      heapStartup, heapOpen, heapClosed,
      startupNetwork: network(requests, 'startup'),
      openNetwork: network(requests, 'open'),
    };
  } finally {
    await context.close();
  }
}

/** Open and close all 30 dialogs three times, unthrottled, and record post-GC heap after release. */
async function memory(browser, origin, fixture) {
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  try {
    const page = await context.newPage();
    const cdp = await context.newCDPSession(page);
    await prepare(page, `${origin}/${fixture.id}/${fixture.variant}/`, fixture.variant);
    const result = { startup: await heap(cdp), cycles: [] };
    for (let cycle = 1; cycle <= 3; cycle++) {
      await page.evaluate(() => window.__bench.cycle());
      await page.waitForTimeout(1_000);
      result.cycles.push({ heap: await heap(cdp), loaded: await page.evaluate(() => window.__bench.loaded()) });
    }
    return result;
  } finally {
    await context.close();
  }
}

function stats(values) {
  const sorted = values.filter(Number.isFinite).sort((a, b) => a - b);
  const at = p => sorted[Math.min(sorted.length - 1, Math.max(0, Math.ceil(p * sorted.length) - 1))];
  return { median: at(0.5), p90: at(0.9), min: sorted[0], max: sorted.at(-1) };
}

function summarize(samples) {
  const summary = {};
  for (const key of Object.keys(samples[0])) {
    if (typeof samples[0][key] === 'number') summary[key] = stats(samples.map(s => s[key]));
    else summary[key] = Object.fromEntries(Object.keys(samples[0][key]).map(k => [k, stats(samples.map(s => s[key][k])).median]));
  }
  return summary;
}

const fixtures = await generate();
const { server, origin } = await serve(fixtures);
const browser = await chromium.launch({ channel: process.env.WEBUI_BENCH_CHANNEL ?? 'msedge' }).catch(() => chromium.launch());
const report = {
  environment: {
    date: new Date().toISOString(), browser: browser.version(), node: process.version, esbuild: esbuildVersion,
    os: `${platform()} ${release()}`, cpu: cpus()[0]?.model, profile: PROFILE_NAME, profileSettings: PROFILE ?? null,
    hoverDwellMs: DWELL_MS, samples: SAMPLES, warmups: WARMUPS, memorySamples: MEMORY_SAMPLES,
    addon: process.env.WEBUI_ADDON_PATH,
  },
  sizes: {}, results: [], memory: [],
};
try {
  for (const fixture of fixtures.values()) {
    const packs = [...fixture.packs.values()];
    report.sizes[`${fixture.id}/${fixture.variant}`] = {
      scriptBrotli: fixture.script.br.length,
      packBrotliMedian: packs.length ? stats(packs.map(pack => pack.br.length)).median : 0,
      packsBrotliTotal: packs.reduce((sum, pack) => sum + pack.br.length, 0),
    };
  }
  const configs = [];
  for (const fixture of fixtures.values()) for (const cache of caches) configs.push({ fixture, cache, samples: [], errors: [] });
  for (let round = 0; round < SAMPLES + WARMUPS; round++) {
    for (const config of round % 2 ? [...configs].reverse() : configs) {
      try {
        const measured = await sample(browser, origin, config.fixture, config.cache);
        if (round >= WARMUPS) config.samples.push(measured);
        process.stdout.write('.');
      } catch (error) {
        config.errors.push(String(error).slice(0, 300));
        process.stdout.write('x');
      }
    }
    process.stdout.write(` ${round + 1}/${SAMPLES + WARMUPS}\n`);
  }
  for (const { fixture, cache, samples, errors } of configs) {
    report.results.push({ scenario: fixture.id, variant: fixture.variant, cache, errors, summary: samples.length ? summarize(samples) : undefined, samples });
  }
  if (scenarios.includes('s30')) {
    for (const variant of variants.filter(name => name === 'plain' || name === 'demand')) {
      const runs = [];
      for (let i = 0; i < MEMORY_SAMPLES; i++) runs.push(await memory(browser, origin, fixtures.get(`s30/${variant}`)));
      report.memory.push({
        variant,
        startup: stats(runs.map(run => run.startup)).median,
        cycles: [0, 1, 2].map(cycle => stats(runs.map(run => run.cycles[cycle].heap)).median),
        loadedAfterRelease: runs.map(run => run.cycles.at(-1).loaded),
        runs,
      });
    }
  }
} finally {
  await browser.close();
  server.close();
  server.closeAllConnections();
}
mkdirSync(dirname(OUTPUT), { recursive: true });
writeFileSync(OUTPUT, JSON.stringify(report, null, 2));
const kib = bytes => (bytes / 1024).toFixed(0);
console.table(report.results.filter(row => row.summary).map(({ scenario, variant, cache, errors, summary: s }) => ({
  scenario, variant, cache,
  startupBytes: s.startupNetwork.bytes, heapKiB: kib(s.heapStartup.median), startupTbt: s.startupTbtMs.median,
  openMs: Math.round(s.openMs.median), openP90: Math.round(s.openMs.p90), waitMs: Math.round(s.waitMs.median),
  reopenMs: Math.round(s.reopenMs.median), openPackBytes: s.openNetwork.bytes, closedKiB: kib(s.heapClosed.median),
  errors: errors.length,
})));
console.table(report.memory.map(({ variant, startup, cycles, loadedAfterRelease }) => ({
  variant, startupKiB: kib(startup), cycle1KiB: kib(cycles[0]), cycle3KiB: kib(cycles[2]),
  loadedAfterRelease: loadedAfterRelease.join(','),
})));
console.table(Object.entries(report.sizes).map(([key, size]) => ({ key, ...size })));
console.log(OUTPUT);
await delay(0);
