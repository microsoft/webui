// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// NTP-shaped comparison of three ways to deliver on-demand UI, modeled on a
// new tab page: a background image as the LCP element, an autofocused search
// box that owns autosuggest, a top-sites grid, four scripted dialogs, and six
// template-heavy menus. The same app is built three ways (see ntp/app.mjs):
//
//   eager   everything in the startup bundle (plain `<if>`)
//   assets  the dialogs as component assets loaded with `import()` and created with
//           `asset.create()`; menus as plain `<if>` (MSN's current approach)
//   lazy    all ten UIs as `<if lazy>` regions; the app imports the generated `lazy.webui.js`
//
// in `--css link` and `--css style`, and driven in Edge with Lighthouse mobile
// throttling. Output: target/lazy-ntp.json with every sample.
//
//   node examples/integration/lazy-ui-bench/ntp.mjs [--samples 20] [--warmups 1]
//        [--configs eager-link,assets-link,assets-style,lazy-link,lazy-style] [--caches cold,warm]
//        [--modes hover,click,prefetch] [--targets settings,context] [--parts open,memory,probes]
//        [--memory-samples 5] [--probe-samples 3] [--profile mobile|none] [--out target/lazy-ntp.json] [--keep]

import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { cpus, platform, release } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { STATE, UIS } from './ntp/app.mjs';
import { buildFixture } from './ntp/build.mjs';
import { PERF_INIT, memoryCycles, prerenderProbe, prime, priorityProbe, visit } from './ntp/measure.mjs';
import { backgroundImage, serve } from './ntp/serve.mjs';
import { compare, stats, verdict } from './ntp/stats.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..', '..');
const require = createRequire(join(root, 'packages', 'webui-test-support', 'package.json'));
const { build: bundle, version: esbuildVersion } = require('esbuild');
const { chromium } = require('@playwright/test');
assert.ok(process.env.WEBUI_ADDON_PATH, 'Set WEBUI_ADDON_PATH to the freshly built release addon.');
const { build, Protocol } = await import(pathToFileURL(join(root, 'packages', 'webui', 'dist', 'index.js')).href);
const { esbuildProjection } = await import(pathToFileURL(join(root, 'packages', 'webui', 'dist', 'projection', 'index.js')).href);

const args = {};
for (let i = 2; i < process.argv.length; i++) {
  const name = process.argv[i];
  if (!name.startsWith('--')) continue;
  const value = process.argv[i + 1];
  args[name.slice(2)] = value === undefined || value.startsWith('--') ? 'true' : value;
}
const list = (value, fallback) => (value ?? fallback).split(',').filter(Boolean);
const SAMPLES = Number(args.samples ?? 20);
const WARMUPS = Number(args.warmups ?? 1);
const MEMORY_SAMPLES = Number(args['memory-samples'] ?? 5);
const PROBE_SAMPLES = Number(args['probe-samples'] ?? 3);
const DWELL_MS = 200;
// Startup lasts until two seconds after hydration and window load, so idle
// work and its long tasks count against startup.
const SETTLE_MS = 2_000;
const PRIMER_TEARDOWN_MS = 1_500;
const HOLD_LOAD_MS = 5_000;
const VIEWPORT = { width: 1280, height: 800 };
const PROFILES = {
  // Lighthouse mobile: 150 ms RTT, 1.6 Mbps down, 750 Kbps up, 4x CPU.
  mobile: { latency: 150, down: 1_600_000 / 8, up: 750_000 / 8, cpu: 4 },
  none: undefined,
};
const PROFILE_NAME = args.profile ?? 'mobile';
const PROFILE = PROFILES[PROFILE_NAME];
const RTT_MS = PROFILE?.latency ?? 0;
const FRAME_MS = 1000 / 60;
const CONFIGS = list(args.configs, 'eager-link,assets-link,assets-style,lazy-link,lazy-style');
const CACHES = list(args.caches, 'cold,warm');
const MODES = list(args.modes, 'hover,click,prefetch');
const TARGETS = list(args.targets, 'settings,context');
const PARTS = new Set(list(args.parts, 'open,memory,probes'));
const OUTPUT = resolve(args.out ?? join(root, 'target', 'lazy-ntp.json'));
/** The probe fixtures give two intent regions the triggers the main app does not use. */
const PROBE_TRIGGERS = { launcher: 'eager', signin: 'idle' };

for (const key of CONFIGS) assert.match(key, /^(eager|assets|lazy)-(link|style)$/, `Unknown config ${key}`);
for (const id of TARGETS) assert.ok(UIS[id] && !UIS[id].noTrigger, `Unknown target ${id}`);
for (const mode of MODES) assert.ok(['hover', 'click', 'prefetch'].includes(mode), `Unknown mode ${mode}`);

const started = Date.now();
mkdirSync(join(root, 'target'), { recursive: true });
// Projection requires sources, packages, and outputs on one filesystem root.
const scratch = mkdtempSync(join(root, 'target', 'lazy-ntp-'));
const keys = [...CONFIGS];
if (PARTS.has('probes')) for (const key of CONFIGS) if (key.startsWith('lazy-')) keys.push(key.replace('lazy-', 'probe-'));
const fixtures = new Map();
try {
  for (const key of keys) {
    const fixture = await buildFixture({
      key, scratch, root, build, Protocol, bundle, esbuildProjection, probeTriggers: PROBE_TRIGGERS,
    });
    for (const warning of fixture.warnings) console.warn(`[${key}] ${warning}`);
    fixtures.set(key, fixture);
  }
} finally {
  if (args.keep === undefined) rmSync(scratch, { recursive: true, force: true });
  else console.log(`Kept fixture sources in ${scratch}`);
}
const built = Date.now();

const browser = await chromium.launch({ channel: process.env.WEBUI_BENCH_CHANNEL ?? 'msedge' }).catch(() => chromium.launch());
const image = await backgroundImage(browser);
const server = await serve(fixtures, STATE, image);
const newContext = async () => {
  const context = await browser.newContext({ viewport: VIEWPORT });
  await context.addInitScript(PERF_INIT);
  return context;
};
const rotate = (values, by) => values.map((_, i) => values[(i + by) % values.length]);

const report = {
  environment: {
    date: new Date().toISOString(), browser: browser.version(), node: process.version, esbuild: esbuildVersion,
    os: `${platform()} ${release()}`, cpu: cpus()[0]?.model, profile: PROFILE_NAME, profileSettings: PROFILE ?? null,
    viewport: VIEWPORT, hoverDwellMs: DWELL_MS, settleMs: SETTLE_MS, samples: SAMPLES, warmups: WARMUPS,
    memorySamples: MEMORY_SAMPLES, probeSamples: PROBE_SAMPLES, configs: CONFIGS, caches: CACHES, modes: MODES,
    targets: TARGETS, imageBytes: image.length, addon: process.env.WEBUI_ADDON_PATH,
  },
  fixtures: {}, runs: [], startup: [], opens: [], memory: [], probes: [], acceptance: [],
};
for (const [key, fixture] of fixtures) {
  report.fixtures[key] = {
    sizes: fixture.sizes, ui: fixture.ui, regions: fixture.regions, lazyScriptsModule: fixture.lazyScriptsModule,
    packs: fixture.packs.map(({ region, owner, trigger, bytes, brotli, scripts, path }) => ({ region, owner, trigger, bytes, brotli, scripts, path })),
  };
}

let pageLoads = 0;
const openStarted = Date.now();
try {
  if (PARTS.has('open')) {
    const combos = CONFIGS.flatMap(key => CACHES.map(cache => ({ key, cache })));
    for (let round = 0; round < SAMPLES + WARMUPS; round++) {
      const targets = round % 2 ? [...TARGETS].reverse() : TARGETS;
      for (const { key, cache } of round % 2 ? [...combos].reverse() : combos) {
        const url = `${server.origin}/${key}/`;
        const record = (mode, result) => {
          if (round >= WARMUPS) report.runs.push({ key, cache, mode, round: round - WARMUPS, ...result });
        };
        let shared;
        try {
          if (cache === 'warm') {
            // A repeat visit: the first visit opened both targets, so their code and templates are cached.
            shared = await newContext();
            await prime(shared, url, TARGETS, PRIMER_TEARDOWN_MS);
          }
          for (const mode of rotate(MODES, round)) {
            const context = shared ?? await newContext();
            try {
              record(mode, await visit({
                context, url, fixture: fixtures.get(key), mode, targets, profile: PROFILE, settleMs: SETTLE_MS, dwellMs: DWELL_MS,
              }));
              process.stdout.write('.');
            } catch (error) {
              record(mode, { error: String(error?.stack ?? error).slice(0, 800) });
              process.stdout.write('x');
            } finally {
              pageLoads++;
              if (shared) await delay(PRIMER_TEARDOWN_MS);
              else await context.close();
            }
          }
        } catch (error) {
          for (const mode of MODES) record(mode, { error: String(error?.stack ?? error).slice(0, 800) });
          process.stdout.write('X');
        } finally {
          await shared?.close();
        }
      }
      process.stdout.write(` ${round + 1}/${SAMPLES + WARMUPS}\n`);
    }
  }
  const openFinished = Date.now();

  if (PARTS.has('memory')) {
    for (const key of CONFIGS) {
      const entry = { key, errors: [], samples: [] };
      for (let i = 0; i < MEMORY_SAMPLES; i++) {
        try {
          entry.samples.push(await memoryCycles({ browser, url: `${server.origin}/${key}/`, viewport: VIEWPORT }));
          process.stdout.write('m');
        } catch (error) {
          entry.errors.push(String(error?.stack ?? error).slice(0, 800));
          process.stdout.write('x');
        }
      }
      report.memory.push(entry);
    }
    process.stdout.write('\n');
  }
  const memoryFinished = Date.now();

  if (PARTS.has('probes')) {
    for (const key of keys.filter(name => !name.startsWith('eager-'))) {
      const fixture = fixtures.get(key);
      const url = `${server.origin}/${key}/`;
      const entry = { key, errors: [], priority: [], prerender: [] };
      for (let i = 0; i < PROBE_SAMPLES; i++) {
        try {
          entry.priority.push(await priorityProbe({ browser, url, fixture, viewport: VIEWPORT, profile: PROFILE, holdMs: HOLD_LOAD_MS }));
          if (fixture.variant === 'lazy') entry.prerender.push(await prerenderProbe({ browser, url, fixture, viewport: VIEWPORT }));
          process.stdout.write('p');
        } catch (error) {
          entry.errors.push(String(error?.stack ?? error).slice(0, 800));
          process.stdout.write('x');
        }
      }
      report.probes.push(entry);
    }
    process.stdout.write('\n');
  }
  report.environment.durations = {
    buildMs: built - started, openMs: openFinished - openStarted, memoryMs: memoryFinished - openFinished,
    probesMs: Date.now() - memoryFinished, pageLoads,
  };
} finally {
  await browser.close();
  server.close();
}

// Summaries.
const ok = report.runs.filter(run => !run.error);
const startupValues = (key, cache, pick) => ok.filter(run => run.key === key && run.cache === cache).map(run => pick(run.startup));
const openValues = (key, cache, id, mode, pick) => ok
  .filter(run => run.key === key && run.cache === cache && run.mode === mode && run.opens[id])
  .map(run => pick(run.opens[id]));
const kindBytes = kind => s => s.network.kinds[kind]?.bytes ?? 0;
const STARTUP_METRICS = {
  fcpMs: s => s.fcpMs, lcpMs: s => s.lcpMs, tbtMs: s => s.tbtMs, hydratedMs: s => s.hydratedMs, loadMs: s => s.loadMs,
  heap: s => s.heap, embedderHeap: s => s.embedderHeap, bytes: s => s.network.bytes, requests: s => s.network.requests,
  documentBytes: kindBytes('document'), scriptBytes: kindBytes('script'), cssBytes: kindBytes('css'),
  imageBytes: kindBytes('image'), packBytes: kindBytes('pack'), chunkBytes: kindBytes('chunk'),
  speculativeRequests: s => s.speculative.length,
};
const OPEN_METRICS = ['openMs', 'visibleMs', 'networkMs', 'waves', 'requests', 'bytes', 'openTbtMs'];
const tally = rows => {
  const counts = {};
  for (const row of rows) counts[row] = (counts[row] ?? 0) + 1;
  return counts;
};
for (const key of CONFIGS) {
  for (const cache of CACHES) {
    const runs = ok.filter(run => run.key === key && run.cache === cache);
    const summary = {};
    for (const [name, pick] of Object.entries(STARTUP_METRICS)) summary[name] = stats(startupValues(key, cache, pick));
    report.startup.push({
      key, cache, summary,
      errors: report.runs.filter(run => run.key === key && run.cache === cache && run.error).length,
      lcpElements: tally(runs.map(run => run.startup.lcpElement)),
      focus: tally(runs.map(run => run.startup.focus)),
      speculative: tally(runs.flatMap(run => run.startup.speculative.map(r => `${r.kind} ${r.priority}${r.beforeLoad ? ' before load' : ''}`))),
    });
    for (const id of TARGETS) {
      for (const mode of MODES) {
        const summary = {};
        for (const metric of OPEN_METRICS) summary[metric] = stats(openValues(key, cache, id, mode, open => open[metric]));
        report.opens.push({ key, cache, id, mode, summary });
      }
    }
  }
}
for (const entry of report.memory) {
  if (entry.samples.length === 0) continue;
  entry.summary = {
    startup: stats(entry.samples.map(sample => sample.startup.js)).median,
    cycles: [0, 1, 2].map(cycle => stats(entry.samples.map(sample => sample.cycles[cycle].js)).median),
    embedder: [0, 1, 2].map(cycle => stats(entry.samples.map(sample => sample.cycles[cycle].embedder)).median),
    loadedRegions: entry.samples.map(sample => sample.cycles.at(-1).loadedRegions),
  };
}
for (const entry of report.probes) {
  const requests = entry.priority.flatMap(run => run.requests);
  entry.summary = {
    beforeLoad: tally(requests.filter(r => r.beforeLoad).map(r => `${r.kind} ${r.priority}`)),
    afterLoad: tally(requests.filter(r => !r.beforeLoad).map(r => `${r.kind} ${r.priority}`)),
    hydratedBeforeLoad: entry.priority.map(run => run.hydratedBeforeLoad),
    quietBeforeLoad: entry.priority.map(run => run.quietBeforeLoad),
    prerenderDuring: entry.prerender.map(run => run.during),
    prerenderAfter: entry.prerender.map(run => run.after),
  };
}

// Acceptance: the new design against component assets and pages without lazy regions.
const kib = bytes => (bytes / 1024).toFixed(0);
const signed = value => `${value >= 0 ? '+' : ''}${value.toFixed(0)}`;
const interval = c => c ? `${signed(c.diff)} [${c.low.toFixed(0)}, ${c.high.toFixed(0)}]` : 'no data';
const medianOf = values => stats(values)?.median;
const accept = (check, numbers, pass) => report.acceptance.push({ check, numbers, pass });
const notWorse = c => c !== undefined && verdict(c) !== 'higher';
for (const css of ['link', 'style']) {
  const lazy = `lazy-${css}`;
  const assets = `assets-${css}`;
  if (!CONFIGS.includes(lazy)) continue;
  const plain = [assets, ...(css === 'link' ? ['eager-link'] : [])].filter(key => CONFIGS.includes(key));
  for (const cache of PARTS.has('open') ? CACHES : []) {
    for (const metric of ['fcpMs', 'lcpMs', 'tbtMs']) {
      for (const base of plain) {
        const c = compare(startupValues(lazy, cache, s => s[metric]), startupValues(base, cache, s => s[metric]));
        accept(`${metric.slice(0, -2).toUpperCase()} ${lazy} vs ${base} (${cache})`, `${interval(c)} ms`, notWorse(c));
      }
    }
    if (!CONFIGS.includes(assets)) continue;
    const bytes = [lazy, assets].map(key => medianOf(startupValues(key, cache, s => s.network.bytes)));
    accept(`Startup bytes ${lazy} <= ${assets} (${cache})`, `${bytes[0]} vs ${bytes[1]} (${signed(bytes[0] - bytes[1])})`, bytes[0] <= bytes[1]);
    const heap = compare(startupValues(lazy, cache, s => s.heap), startupValues(assets, cache, s => s.heap));
    const heaps = [lazy, assets].map(key => medianOf(startupValues(key, cache, s => s.heap)));
    accept(`Startup JS heap ${lazy} <= ${assets} (${cache})`, `${kib(heaps[0])} vs ${kib(heaps[1])} KiB, diff ${interval(heap && { diff: heap.diff / 1024, low: heap.low / 1024, high: heap.high / 1024 })} KiB`, notWorse(heap));
    for (const id of TARGETS) {
      for (const mode of ['hover', 'prefetch'].filter(name => MODES.includes(name))) {
        const c = compare(openValues(lazy, cache, id, mode, o => o.openMs), openValues(assets, cache, id, mode, o => o.openMs));
        accept(`${mode === 'hover' ? 'Hover-then-click' : 'Code-prefetch'} open of ${id}, ${lazy} vs ${assets} (${cache})`, `${interval(c)} ms`, notWorse(c));
      }
      if (MODES.includes('click')) {
        // One round trip for the pack and scripts in parallel, a second for CSS
        // files in link mode, plus the time to transfer the bytes and two frames.
        const [lazyOpen, assetsOpen] = [lazy, assets].map(key => medianOf(openValues(key, cache, id, 'click', o => o.openMs)));
        const waves = medianOf(openValues(lazy, cache, id, 'click', o => o.waves));
        const bytes = medianOf(openValues(lazy, cache, id, 'click', o => o.bytes)) ?? 0;
        const trips = css === 'link' && fixtures.get(lazy).ui[id].css.length > 0 ? 2 : 1;
        const allowance = trips * RTT_MS + (PROFILE ? (bytes / PROFILE.down) * 1000 : 0) + 2 * FRAME_MS;
        const extra = lazyOpen - assetsOpen;
        accept(`Click-only open of ${id}, ${lazy} within ${trips === 1 ? 'one round trip' : 'two round trips'} of ${assets} (${cache})`,
          `${lazyOpen?.toFixed(0)} vs ${assetsOpen?.toFixed(0)} ms (${signed(extra)} ms, allowance ${allowance.toFixed(0)} ms; ${waves} network waves)`,
          Number.isFinite(extra) && extra <= allowance);
        if (cache === 'cold') {
          const assetsWaves = medianOf(openValues(assets, cache, id, 'click', o => o.waves));
          accept(`Click-only open of ${id}, ${lazy} fetches pack, scripts, and CSS in one network round (${cache})`,
            `${waves} vs ${assetsWaves} sequential rounds (median)`, waves !== undefined && waves <= 1);
        }
      }
    }
  }
  const lazyMemory = report.memory.find(entry => entry.key === lazy)?.summary;
  const assetsMemory = report.memory.find(entry => entry.key === assets)?.summary;
  if (lazyMemory && assetsMemory) {
    const [after, baseline] = [lazyMemory.cycles[2], assetsMemory.cycles[2]];
    const change = `${kib(after)} vs ${kib(baseline)} KiB (${signed((after / baseline - 1) * 100)}%)`;
    accept(`JS heap after three cycles, ${lazy} below ${assets}`, change, after < baseline);
    accept(`JS heap after three cycles, ${lazy} at least 10% below ${assets}`, change, after <= baseline * 0.9);
  }
  for (const key of [lazy, `probe-${css}`]) {
    const probe = report.probes.find(entry => entry.key === key);
    if (!probe) continue;
    const during = probe.prerender.map(run => run.during);
    accept(`No speculative requests while prerendering, ${key}`,
      `${during.join(', ')} during; ${probe.prerender.map(run => run.after).join(', ')} after activation`,
      during.length > 0 && during.every(count => count === 0));
    const before = [
      ...probe.priority.flatMap(run => run.requests),
      ...ok.filter(run => run.key === key).flatMap(run => run.startup.speculative),
    ].filter(request => request.beforeLoad && !request.cached);
    const high = before.filter(request => request.priority === 'High' || request.priority === 'VeryHigh');
    accept(`No high-priority speculative requests before window load, ${key}`,
      `${high.length} of ${before.length} high: ${JSON.stringify(tally(before.map(r => `${r.kind} ${r.priority}`)))}`,
      before.length > 0 && high.length === 0);
  }
}

mkdirSync(dirname(OUTPUT), { recursive: true });
writeFileSync(OUTPUT, JSON.stringify(report, null, 2));

// Console summary.
const round0 = value => value === undefined ? '-' : Math.round(value);
console.log('\nFixture sizes (Brotli bytes; HTML at quality 5, static files at 11)');
console.table([...fixtures.values()].map(({ key, sizes }) => ({
  key, html: sizes.htmlBrotli5, startupJs: sizes.startupScriptsBrotli, jsFiles: sizes.startupScripts,
  startupCss: sizes.startupCssBrotli, packs: sizes.packsBrotli, duplicatedPackBr: sizes.duplicated.brotli,
  duplicatedPackRaw: sizes.duplicated.raw,
})));
console.log('Deferred bytes per UI (Brotli): pack + chunks + CSS files');
console.table([...fixtures.values()].filter(f => f.variant !== 'eager' && !f.key.startsWith('probe')).flatMap(({ key, ui }) =>
  Object.entries(ui).filter(([, entry]) => entry.pack || entry.chunkBytes).map(([id, entry]) => ({
    key, id, pack: entry.pack, chunks: entry.chunkBytes, chunkFiles: entry.chunks.length, css: entry.cssBytes,
    total: entry.pack + entry.chunkBytes + entry.cssBytes,
  }))));
if (report.startup.length) {
  console.log('Startup (median / p90; bytes and requests are medians)');
  console.table(report.startup.map(({ key, cache, summary: s, errors }) => ({
    key, cache, fcp: `${round0(s.fcpMs?.median)} / ${round0(s.fcpMs?.p90)}`, lcp: `${round0(s.lcpMs?.median)} / ${round0(s.lcpMs?.p90)}`,
    tbt: `${round0(s.tbtMs?.median)} / ${round0(s.tbtMs?.p90)}`, hydrated: round0(s.hydratedMs?.median),
    bytes: s.bytes?.median, requests: s.requests?.median, jsHeapKiB: s.heap ? kib(s.heap.median) : '-',
    speculative: s.speculativeRequests?.median, errors,
  })));
  console.log('Opens: input to painted, visually complete (median / p90 ms)');
  console.table(report.opens.map(({ key, cache, id, mode, summary: s }) => ({
    key, cache, id, mode, open: `${round0(s.openMs?.median)} / ${round0(s.openMs?.p90)}`,
    network: round0(s.networkMs?.median), waves: s.waves?.median, requests: s.requests?.median, bytes: s.bytes?.median,
    tbt: round0(s.openTbtMs?.median),
  })));
}
if (report.memory.length) {
  console.log('JS heap after open/close cycles of all ten UIs (median KiB, unthrottled)');
  console.table(report.memory.filter(entry => entry.summary).map(({ key, summary, errors }) => ({
    key, startup: kib(summary.startup), cycle1: kib(summary.cycles[0]), cycle3: kib(summary.cycles[2]),
    embedderCycle3: kib(summary.embedder[2]), regionsLoadedAfter: summary.loadedRegions.join(','), errors: errors.length,
  })));
}
if (report.probes.length) {
  console.log('Speculative requests by initial priority (priority probe) and while prerendering');
  console.table(report.probes.map(({ key, summary, errors }) => ({
    key, beforeLoad: JSON.stringify(summary.beforeLoad), afterLoad: JSON.stringify(summary.afterLoad),
    prerenderDuring: summary.prerenderDuring.join(','), prerenderAfter: summary.prerenderAfter.join(','), errors: errors.length,
  })));
}
console.log('Acceptance');
for (const { check, numbers, pass } of report.acceptance) console.log(`  ${pass ? 'PASS' : 'FAIL'}  ${check}: ${numbers}`);
const failures = report.runs.filter(run => run.error);
if (failures.length) console.log(`${failures.length} failed page loads; first: ${failures[0].error}`);
const { durations } = report.environment;
const perLoad = durations.pageLoads ? durations.openMs / durations.pageLoads : 0;
const fullLoads = 21 * 5 * 2 * 3;
console.log(`Build ${(durations.buildMs / 1000).toFixed(0)} s; open runs ${(durations.openMs / 1000).toFixed(0)} s over ${durations.pageLoads} page loads `
  + `(${(perLoad / 1000).toFixed(1)} s each); memory ${(durations.memoryMs / 1000).toFixed(0)} s; probes ${(durations.probesMs / 1000).toFixed(0)} s.`);
if (perLoad) console.log(`Full default run: about ${((fullLoads * perLoad) / 60_000).toFixed(0)} min of page loads for ${fullLoads} loads, plus memory and probes.`);
console.log(OUTPUT);
await delay(0);