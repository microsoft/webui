// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { cpus, platform, release } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { esbuildVersion, modes, startFixtures } from './fixture.mjs';
import { installResponsivenessProbe, summarize } from './responsiveness-metrics.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..', '..');
const require = createRequire(join(root, 'package.json'));
const { chromium } = require('@playwright/test');
const samples = Number(process.env.WEBUI_BENCH_SAMPLES ?? 20);
const latencyMs = Number(process.env.WEBUI_BENCH_LATENCY_MS ?? 40);
const counts = [250, 1000];
const cpuRates = [1, 4];
assert.ok(Number.isSafeInteger(samples) && samples >= 3 && samples <= 100);
assert.ok(Number.isFinite(latencyMs) && latencyMs >= 0 && latencyMs <= 1000);
assert.ok(process.env.WEBUI_ADDON_PATH, 'Select the current release native addon.');

async function click(page, point, phase) {
  await page.evaluate(name => window.uiProbe.arm(name), phase);
  await page.mouse.click(point.x + point.width / 2, point.y + point.height / 2);
}

async function finished(page, phase) {
  await page.waitForFunction(name => window.uiProbe.actions.some(action =>
    action.name === name && action.complete), phase);
}

async function checkInputs(page, count, value) {
  const values = await page.locator('bench-owner section input').evaluateAll(inputs => inputs.map(input => input.value));
  assert.equal(values.length, count);
  assert.ok(values.every(actual => actual === value), 'the reactive update must reach every control');
}

async function run(browser, origin, mode, count, cpuRate) {
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  const page = await context.newPage();
  const session = await context.newCDPSession(page);
  const errors = [];
  page.on('pageerror', error => errors.push(String(error)));
  try {
    await session.send('Emulation.setCPUThrottlingRate', { rate: cpuRate });
    await page.addInitScript(installResponsivenessProbe);
    await page.goto(`${origin}/${mode}/${count}/`);
    await page.waitForFunction(() => window.bench && document.querySelector('bench-owner').$ready);
    const open = await page.getByRole('button', { name: 'Open panel', exact: true }).boundingBox();
    const shell = await page.getByRole('button', { name: 'Shell click', exact: true }).boundingBox();
    const close = await page.getByRole('button', { name: 'Close panel', exact: true }).boundingBox();
    assert.ok(open && shell && close);
    await click(page, open, 'cold-open');
    await click(page, shell, 'shell-during-open');
    await finished(page, 'cold-open');
    await finished(page, 'shell-during-open');
    await checkInputs(page, count, 'Fresh');
    assert.equal(await page.locator('bench-owner .shell-count').textContent(), '1');

    const input = page.locator('bench-owner section input').first();
    await input.click();
    await input.press('End');
    let value = 'Fresh';
    for (const character of ['a', 'b', 'c', 'd', 'e']) {
      const phase = `typing-local-${character}`;
      await page.evaluate(name => window.uiProbe.arm(name), phase);
      await input.press(character);
      await finished(page, phase);
      value += character;
      assert.equal(await input.inputValue(), value);
      assert.equal(await page.locator('bench-owner .edit-preview').textContent(), value);
      const untouched = await page.locator('bench-owner section input').evaluateAll(inputs =>
        inputs.slice(1).every(input => input.value === 'Fresh'));
      assert.ok(untouched, 'single-field edits must not update other controls');
    }
    await page.getByLabel('Broadcast edits').check();
    await input.click();
    await input.press('End');
    for (const character of ['a', 'b', 'c', 'd', 'e']) {
      const phase = `typing-fanout-${character}`;
      await page.evaluate(name => window.uiProbe.arm(name), phase);
      await input.press(character);
      await finished(page, phase);
      value += character;
      await checkInputs(page, count, value);
    }
    await click(page, close, 'close');
    await finished(page, 'close');
    await checkInputs(page, 0, '');
    await click(page, open, 'warm-open');
    await finished(page, 'warm-open');
    await checkInputs(page, count, mode === 'eager-local' ? value : 'Fresh');
    // Event Timing notifications may be delivered after the rendering callbacks.
    await page.waitForTimeout(150);
    const actions = await page.evaluate(() => window.uiProbe.read());
    assert.equal(actions.length, 14);
    assert.ok(actions.every(action => action.trusted));
    assert.deepEqual(errors, []);
    return actions;
  } finally {
    await session.detach();
    await context.close();
  }
}

const server = await startFixtures(counts, latencyMs, true);
let browser;
const raw = [];
try {
  browser = await chromium.launch();
  for (let iteration = -2; iteration < samples; iteration++) {
    const order = iteration % 2 === 0 ? modes : [...modes].reverse();
    const rates = iteration % 2 === 0 ? cpuRates : [...cpuRates].reverse();
    for (const cpuRate of rates) for (const controls of counts) for (const mode of order) {
      const actions = await run(browser, server.origin, mode, controls, cpuRate);
      if (iteration >= 0) raw.push({ iteration, cpuRate, controls, mode, actions });
    }
  }
  const results = [];
  for (const cpuRate of cpuRates) for (const controls of counts) for (const mode of modes) {
    const trials = raw.filter(row => row.cpuRate === cpuRate && row.controls === controls && row.mode === mode);
    for (const phase of ['cold-open', 'shell-during-open', 'typing-local', 'typing-fanout', 'close', 'warm-open']) {
      const actions = trials.flatMap(trial => trial.actions.filter(action =>
        phase.startsWith('typing-') ? action.name.startsWith(`${phase}-`) : action.name === phase));
      results.push({
        cpuRate, controls, mode, phase,
        events: summarize(actions.map(action => action.eventTimingMs), 16),
        inputDelayMs: summarize(actions.map(action => action.inputDelayMs)),
        domReadyMs: summarize(actions.map(action => action.domReadyMs)),
        twoRafReadyMs: summarize(actions.map(action => action.twoRafReadyMs)),
        maxFrameGapMs: summarize(actions.map(action => action.maxFrameGapMs)),
        maxLongTaskMs: summarize(actions.map(action => action.maxLongTaskMs)),
        interactionsWithLongTasks: actions.filter(action => action.longTaskCount > 0).length,
        interactionsWithFrameGapsOver50Ms: actions.filter(action => action.frameGapsOver50Ms > 0).length,
        pendingAtInput: actions.filter(action => action.pendingAtInput).length,
      });
    }
  }
  const report = {
    environment: {
      node: process.version, chromium: browser.version(), esbuild: esbuildVersion,
      os: `${platform()} ${release()}`, cpu: cpus()[0].model,
      samples, discardedWarmups: 2, stateDelayMs: latencyMs, cpuRates,
    },
    methodology: 'Trusted mouse clicks and keyboard events; single-field typing changes the native field and a reactive preview, while broadcast typing updates all 250/1000 value bindings as a separate stress case. Event Timing is browser-reported click/keydown-to-next-paint, quantized to 8 ms and censored below 16 ms. This is not field INP. Async form readiness uses a separate two-rAF render-opportunity proxy, not measured pixel presentation. Long tasks and rAF frame gaps are recorded through each action. No forced GC during interactions. CPU throttling is simulation, not a measured low-end device.',
    results, trials: raw,
  };
  const output = resolve(process.argv[2] ?? join(root, 'target', 'lazy-responsiveness.json'));
  await writeFile(output, JSON.stringify(report, null, 2) + '\n');
  const value = item => `${item.censored ? '<' : ''}${item.value.toFixed(1)}`;
  console.table(results.filter(row => row.phase !== 'close').map(row => ({
    cpu: row.cpuRate, controls: row.controls, mode: row.mode, action: row.phase,
    eventP50: value(row.events.median), eventP95: value(row.events.p95),
    readyFrameP50: value(row.twoRafReadyMs.median),
    readyFrameP95: value(row.twoRafReadyMs.p95),
    longTaskWorst: value(row.maxLongTaskMs.worst),
    gapWorst: value(row.maxFrameGapMs.worst),
    pending: row.pendingAtInput,
  })));
  console.log(output);
} finally {
  await browser?.close();
  await server.close();
}
