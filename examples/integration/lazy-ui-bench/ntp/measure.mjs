// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// Browser side of the NTP benchmark: request tracking, startup metrics, opens
// driven by real pointer input, memory cycles, and the priority and prerender
// probes.

import { setTimeout as delay } from 'node:timers/promises';
import { UIS } from './app.mjs';

/** Observers registered before any page script, so startup entries are complete. */
export const PERF_INIT = `(() => {
  const log = { longTasks: [], fcp: 0, lcp: 0, lcpUrl: '', lcpSize: 0, lcpElement: '' };
  log.ready = new Promise(resolve => { log.resolveReady = resolve; });
  window.__perf = log;
  const observe = (type, record) => {
    try {
      new PerformanceObserver(list => { for (const entry of list.getEntries()) record(entry); }).observe({ type, buffered: true });
    } catch {}
  };
  observe('longtask', entry => log.longTasks.push([entry.startTime, entry.duration]));
  observe('paint', entry => { if (entry.name === 'first-contentful-paint') log.fcp = entry.startTime; });
  observe('largest-contentful-paint', entry => {
    log.lcp = entry.startTime;
    log.lcpUrl = entry.url || '';
    log.lcpSize = entry.size;
    log.lcpElement = entry.element ? entry.element.localName + (entry.element.className ? '.' + entry.element.className : '') : '';
  });
})();`;

/** Report `document.prerendering` as true until the driver dispatches `prerenderingchange`. */
export const PRERENDER_INIT = `(() => {
  let prerendering = true;
  Object.defineProperty(Document.prototype, 'prerendering', { configurable: true, get() { return prerendering; } });
  window.__activatePrerender = () => {
    prerendering = false;
    document.dispatchEvent(new Event('prerenderingchange'));
  };
})();`;

const SPECULATIVE = new Set(['pack', 'chunk', 'deferred-css']);
const pathOf = url => new URL(url).pathname.split('/').slice(2).join('/');

/** Classify a request by what it delivers. */
export function kindOf(url, fixture) {
  // Fixture files live under /<key>/; anything else, such as /favicon.ico, is not the page's.
  if (new URL(url).pathname.split('/').length < 3) return 'other';
  const path = pathOf(url);
  if (path === '') return 'document';
  if (path.startsWith('_webui/packs/')) return 'pack';
  if (path === 'bg.jpg') return 'image';
  if (path.endsWith('.js')) return fixture.startupScripts.has(path) ? 'script' : 'chunk';
  if (path.endsWith('.css')) return fixture.startupCss.has(path) ? 'css' : 'deferred-css';
  return 'other';
}

/** Record every request with the phase that issued it. */
function track(cdp) {
  const net = { phase: 'startup', requests: new Map(), inflight: new Set(), loadAt: Infinity };
  const done = id => net.inflight.delete(id);
  cdp.on('Network.requestWillBeSent', event => {
    const { url } = event.request;
    if (!url.startsWith('http')) return;
    net.requests.set(event.requestId, {
      url, type: event.type, priority: event.request.initialPriority, start: event.timestamp,
      phase: net.phase, bytes: 0, cached: false, status: 0,
    });
    if (!url.includes('__hold.gif')) net.inflight.add(event.requestId);
  });
  cdp.on('Network.requestServedFromCache', event => {
    const request = net.requests.get(event.requestId);
    if (request) request.cached = true;
  });
  cdp.on('Network.responseReceived', event => {
    const request = net.requests.get(event.requestId);
    if (!request) return;
    request.status = event.response.status;
    request.cached ||= Boolean(event.response.fromDiskCache || event.response.fromPrefetchCache);
  });
  cdp.on('Network.loadingFinished', event => {
    const request = net.requests.get(event.requestId);
    if (request) request.bytes = event.encodedDataLength;
    done(event.requestId);
  });
  cdp.on('Network.loadingFailed', event => {
    const request = net.requests.get(event.requestId);
    if (request) request.failed = event.errorText;
    done(event.requestId);
  });
  cdp.on('Page.loadEventFired', event => {
    net.loadAt = event.timestamp;
  });
  return net;
}

/** Requests, bytes off the network, and cache hits for the phases `include` accepts. */
export function network(net, fixture, include) {
  const result = { requests: 0, bytes: 0, cached: 0, kinds: {} };
  for (const request of net.requests.values()) {
    if (!include(request.phase)) continue;
    const kind = kindOf(request.url, fixture);
    if (kind === 'other') continue;
    const bucket = result.kinds[kind] ??= { requests: 0, bytes: 0 };
    result.requests++;
    bucket.requests++;
    if (request.cached) result.cached++;
    else {
      result.bytes += request.bytes;
      bucket.bytes += request.bytes;
    }
  }
  return result;
}

/**
 * When each request started, on the page's own clock: Resource Timing start
 * times and the navigation's `loadEventStart`. CDP reports the load event only
 * after its listeners ran, so a request a `load` listener starts would look
 * earlier than the event on the CDP clock.
 */
export async function pageClock(page) {
  return page.evaluate(() => {
    const starts = {};
    for (const entry of performance.getEntriesByType('resource')) starts[entry.name] ??= entry.startTime;
    return { loadStart: performance.getEntriesByType('navigation')[0]?.loadEventStart ?? 0, starts };
  });
}

/** Pack, chunk, and deferred CSS requests with their initial priority and timing relative to `load`. */
export function speculativeRequests(net, fixture, clock) {
  const rows = [];
  for (const request of net.requests.values()) {
    const kind = kindOf(request.url, fixture);
    if (!SPECULATIVE.has(kind)) continue;
    const start = clock?.starts[request.url];
    const beforeLoad = !clock?.loadStart ? request.start < net.loadAt
      : start !== undefined ? start < clock.loadStart : request.start < net.loadAt;
    rows.push({
      kind, path: pathOf(request.url), priority: request.priority, phase: request.phase,
      beforeLoad, cached: request.cached,
    });
  }
  return rows;
}

/** Resolve once no request has been in flight for `ms`. */
export async function quiet(net, ms = 300, timeout = 20_000) {
  const start = Date.now();
  let since = Date.now();
  while (Date.now() - start < timeout) {
    if (net.inflight.size > 0) since = Date.now();
    else if (Date.now() - since >= ms) return;
    await delay(25);
  }
  throw new Error(`Network did not go quiet: ${[...net.inflight].map(id => net.requests.get(id)?.url).join(', ')}`);
}

export async function newPage(context, profile) {
  const page = await context.newPage();
  const cdp = await context.newCDPSession(page);
  const net = track(cdp);
  await cdp.send('Network.enable');
  await cdp.send('Page.enable');
  if (profile) {
    await cdp.send('Network.emulateNetworkConditions', {
      offline: false, latency: profile.latency, downloadThroughput: profile.down, uploadThroughput: profile.up,
    });
    await cdp.send('Emulation.setCPUThrottlingRate', { rate: profile.cpu });
  }
  return { page, cdp, net };
}

function withTimeout(promise, ms, label) {
  let timer;
  return Promise.race([
    promise.finally(() => clearTimeout(timer)),
    new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`Timed out: ${label}`)), ms); }),
  ]);
}

/** Navigate and resolve once every owner has hydrated and the window has loaded. */
export async function load(page, url) {
  await page.goto(url, { waitUntil: 'commit', timeout: 60_000 });
  await page.waitForFunction(() => window.__perf !== undefined, null, { polling: 50, timeout: 60_000 });
  await withTimeout(page.evaluate(() => window.__perf.ready), 60_000, `ready ${url}`);
}

/** Post-GC JS heap and embedder (DOM) heap. */
export async function heap(cdp) {
  await cdp.send('HeapProfiler.collectGarbage');
  await cdp.send('HeapProfiler.collectGarbage');
  const usage = await cdp.send('Runtime.getHeapUsage');
  return { js: usage.usedSize, embedder: usage.embedderHeapUsedSize ?? 0 };
}

/** Lighthouse-style blocking time: each long task's time beyond 50 ms that falls inside [from, to]. */
export function blockingTime(tasks, from, to) {
  let total = 0;
  for (const [start, duration] of tasks) {
    const blockedFrom = Math.max(start + 50, from);
    const blockedTo = Math.min(start + duration, to);
    if (blockedTo > blockedFrom) total += blockedTo - blockedFrom;
  }
  return total;
}

/** Network round trips in sequence: a request starting after another's response ended is one wave later. */
export function waves(rows) {
  const sorted = rows.slice().sort((a, b) => a[1] - b[1]);
  const depth = [];
  let deepest = 0;
  for (let i = 0; i < sorted.length; i++) {
    let wave = 1;
    for (let j = 0; j < i; j++) if (sorted[j][2] <= sorted[i][1] + 1) wave = Math.max(wave, depth[j] + 1);
    depth.push(wave);
    deepest = Math.max(deepest, wave);
  }
  return deepest;
}

const idle = page => page.evaluate(() => new Promise(resolve => requestIdleCallback(() => resolve(), { timeout: 2000 })));

/** Open one UI with real pointer input in one of three ways, then close it. */
async function open(page, net, fixture, id, mode, dwellMs) {
  if (mode === 'prefetch') {
    net.phase = `prefetch:${id}`;
    await page.evaluate(ui => window.__bench.prefetch(ui), id);
    await page.waitForFunction(ui => window.__bench.prefetched(ui), id, { polling: 50, timeout: 30_000 });
    await quiet(net);
  }
  const point = await page.evaluate(ui => window.__bench.point(ui), id);
  if (!point) throw new Error(`No trigger for ${id}`);
  await page.evaluate(ui => window.__bench.arm(ui), id);
  if (mode === 'hover') {
    net.phase = `hover:${id}`;
    await page.mouse.move(point.x, point.y);
    await delay(dwellMs);
  }
  net.phase = `open:${id}`;
  if (mode !== 'hover') await page.mouse.move(point.x, point.y);
  await page.mouse.down();
  await page.mouse.up();
  const result = await withTimeout(page.evaluate(() => window.__bench.result()), 40_000, `open ${id}`);
  if ('error' in result) throw new Error(result.error);
  await quiet(net);
  const transfer = network(net, fixture, phase => phase === `open:${id}` || phase === `hover:${id}`);
  net.phase = `close:${id}`;
  await page.evaluate(ui => window.__bench.close(ui), id);
  await idle(page);
  await delay(300);
  const lastResponse = result.resources.reduce((latest, row) => Math.max(latest, row[2]), result.inputAt);
  return {
    openMs: result.paintedAt - result.inputAt,
    visibleMs: result.visibleAt - result.inputAt,
    networkMs: lastResponse - result.inputAt,
    // HTTP cache hits transfer nothing and take no round trip.
    waves: waves(result.resources.filter(row => row[3] > 0)),
    requests: transfer.requests,
    bytes: transfer.bytes,
    kinds: transfer.kinds,
    openTbtMs: blockingTime(result.longTasks, result.inputAt, result.paintedAt),
  };
}

/** One measured page load: startup metrics, then each target opened in `mode`. */
export async function visit({ context, url, fixture, mode, targets, profile, settleMs, dwellMs }) {
  const { page, cdp, net } = await newPage(context, profile);
  try {
    await load(page, url);
    await delay(settleMs);
    const page0 = await page.evaluate(() => {
      const navigation = performance.getEntriesByType('navigation')[0];
      return {
        fcp: window.__perf.fcp, lcp: window.__perf.lcp, lcpUrl: window.__perf.lcpUrl, lcpElement: window.__perf.lcpElement,
        longTasks: window.__perf.longTasks.slice(), hydratedAt: window.__bench.hydratedAt,
        load: navigation.loadEventEnd, domContentLoaded: navigation.domContentLoadedEventEnd,
        focus: window.__bench.focusPath(), loadedRegions: window.__bench.loadedRegions(),
      };
    });
    const memory = await heap(cdp);
    const clock = await pageClock(page);
    const startup = {
      fcpMs: page0.fcp,
      lcpMs: page0.lcp,
      tbtMs: blockingTime(page0.longTasks, page0.fcp, Math.max(page0.hydratedAt, page0.load) + settleMs),
      hydratedMs: page0.hydratedAt,
      loadMs: page0.load,
      heap: memory.js,
      embedderHeap: memory.embedder,
      lcpElement: page0.lcpElement,
      lcpUrl: page0.lcpUrl,
      focus: page0.focus,
      loadedRegions: page0.loadedRegions,
      network: network(net, fixture, phase => phase === 'startup'),
      speculative: speculativeRequests(net, fixture, clock).filter(request => request.phase === 'startup'),
    };
    const opens = {};
    for (const id of targets) opens[id] = await open(page, net, fixture, id, mode, dwellMs);
    return { startup, opens };
  } finally {
    await page.close();
  }
}

/** Warm the HTTP cache: load the page and open each target once, then let the tab tear down. */
export async function prime(context, url, targets, teardownMs) {
  const page = await context.newPage();
  await load(page, url);
  for (const id of targets) {
    await page.evaluate(ui => window.__bench.open(ui), id);
    await page.evaluate(ui => window.__bench.close(ui), id);
  }
  await delay(500);
  await page.close();
  // Chromium tears a closed tab's compositor down about a second later on the
  // renderer thread that same-origin pages share.
  await delay(teardownMs);
}

/** Open and close all ten UIs three times, unthrottled, reading post-GC heap after each cycle. */
export async function memoryCycles({ browser, url, viewport }) {
  const context = await browser.newContext({ viewport });
  await context.addInitScript(PERF_INIT);
  try {
    const { page, cdp } = await newPage(context);
    await load(page, url);
    await delay(1_000);
    const startup = await heap(cdp);
    const cycles = [];
    for (let cycle = 0; cycle < 3; cycle++) {
      for (const id of Object.keys(UIS)) {
        await page.evaluate(ui => window.__bench.open(ui), id);
        await page.evaluate(ui => window.__bench.close(ui), id);
        await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => setTimeout(resolve, 0))));
      }
      // Closed regions are released at idle.
      await delay(1_000);
      cycles.push({ ...(await heap(cdp)), loadedRegions: await page.evaluate(() => window.__bench.loadedRegions()) });
    }
    return { startup, cycles };
  } finally {
    await context.close();
  }
}

/**
 * Hover every trigger and prefetch from code while the window `load` event is
 * held back, then hover one more trigger after `load`, recording each
 * speculative request's initial priority.
 */
export async function priorityProbe({ browser, url, fixture, viewport, profile, holdMs }) {
  const context = await browser.newContext({ viewport });
  await context.addInitScript(PERF_INIT);
  try {
    const { page, net } = await newPage(context, profile);
    await page.goto(`${url}?hold=${holdMs}`, { waitUntil: 'commit' });
    await page.waitForFunction(() => (window.__bench?.hydratedAt ?? 0) > 0, null, { polling: 50, timeout: 60_000 });
    const hydratedBeforeLoad = await page.evaluate(() => document.readyState !== 'complete');
    net.phase = 'input-before-load';
    const later = 'gallery';
    for (const id of Object.keys(UIS)) {
      if (id === later) continue;
      const point = await page.evaluate(ui => window.__bench.point(ui), id);
      if (!point) continue;
      await page.mouse.move(point.x, point.y);
      await delay(80);
    }
    await page.evaluate(() => window.__bench.prefetch('toast'));
    await quiet(net, 500);
    const quietBeforeLoad = await page.evaluate(() => document.readyState !== 'complete');
    await page.waitForFunction(() => document.readyState === 'complete', null, { polling: 100, timeout: 60_000 });
    await delay(1_500);
    net.phase = 'input-after-load';
    const point = await page.evaluate(ui => window.__bench.point(ui), later);
    await page.mouse.move(point.x, point.y);
    await delay(100);
    await quiet(net, 500);
    return { hydratedBeforeLoad, quietBeforeLoad, requests: speculativeRequests(net, fixture, await pageClock(page)) };
  } finally {
    await context.close();
  }
}

/**
 * Load with `document.prerendering` forced true, let eager and idle triggers
 * come due, hover every trigger and prefetch every UI from code, then
 * activate. Nothing speculative may load before activation.
 */
export async function prerenderProbe({ browser, url, fixture, viewport }) {
  const context = await browser.newContext({ viewport });
  await context.addInitScript(PRERENDER_INIT);
  await context.addInitScript(PERF_INIT);
  try {
    const { page, net } = await newPage(context);
    await load(page, url);
    await delay(2_500);
    net.phase = 'prerendering-input';
    for (const id of Object.keys(UIS)) {
      const point = await page.evaluate(ui => window.__bench.point(ui), id);
      if (point) await page.mouse.move(point.x, point.y);
      await page.evaluate(ui => window.__bench.prefetch(ui), id);
      await delay(50);
    }
    await delay(1_500);
    const during = speculativeRequests(net, fixture);
    net.phase = 'activated';
    await page.evaluate(() => window.__activatePrerender());
    await delay(500);
    await quiet(net, 500);
    const after = speculativeRequests(net, fixture).filter(request => request.phase === 'activated');
    return { during: during.length, duringRequests: during, after: after.length };
  } finally {
    await context.close();
  }
}
