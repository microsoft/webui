// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { chromium, expect } from '@playwright/test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import type { BenchmarkPreview } from '../../../docs/.webui-press/components/benchmark-preview/benchmark-preview-data.js';
import { contrastRatio, docsDist, startWebsite } from './website-fixture.js';

declare global {
  interface Window {
    installTestCopies: string[];
    finishInstallTestCopy?: () => void;
  }
}

const commands = [
  ['npm', 'npm install -D @microsoft/webui'],
  ['pnpm', 'pnpm add -D @microsoft/webui'],
  ['Yarn', 'yarn add -D @microsoft/webui'],
] as const;

test('the entry page serves the install command and qualified fixed-case preview without JavaScript', async t => {
  const server = await startWebsite();
  t.after(() => server.close());
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage({ javaScriptEnabled: false, viewport: { width: 390, height: 844 } });
  await page.goto(`${server.origin}/webui/`);
  const install = page.locator('install-command');
  await expect(install.locator('code')).toHaveText(commands[0][1]);
  await expect(install.getByRole('status')).toBeEmpty();
  await expect(install.getByRole('button', { name: 'Copy install command' })).toBeDisabled();
  await expect(install.getByRole('radio', { name: 'npm', exact: true })).toBeChecked();
  for (const [label] of commands) {
    await expect(install.getByRole('radio', { name: label, exact: true })).toBeDisabled();
  }
  const preview = page.locator('benchmark-preview');
  await expect(preview.getByRole('heading', { name: 'Benchmark snapshot' })).toBeVisible();
  await expect(preview.getByRole('tab')).toHaveCount(4);
  for (const tab of await preview.getByRole('tab').all()) await expect(tab).toBeDisabled();
  await expect(preview.getByRole('tab', { name: 'RPS', exact: true })).toHaveAttribute('aria-selected', 'true');
  await expect(preview.locator('tbody tr')).toHaveCount(3);
  assert.deepEqual(await preview.locator('tbody th').allTextContents(), [
    'WebUI', 'React', 'Next.js',
  ]);
  assert.deepEqual(await preview.locator('.value').allTextContents(), ['1,741.15 req/s', '490.55 req/s', '151.2 req/s']);
  await expect(preview.locator('tbody tr[data-webui]')).toHaveAttribute('data-case', 'webui-rust-actix');
  await expect(preview.locator('.chart-caption')).toContainText('Complete responses (req/s)');
  await expect(preview.locator('.direction')).toHaveText('Higher is better');
  await expect(page.locator('.benchmark-notes')).toHaveCount(0);
  await expect(preview.locator('.scale')).toContainText('1,741.15');
  const widths = await preview.locator('.bar').evaluateAll(elements =>
    elements.map(element => Number(element.getAttribute('width'))));
  for (const [index, expected] of [100, 28.173908049277774, 8.683915802774028].entries()) {
    assert.ok(Math.abs(widths[index] - expected) < 1e-10, 'SSR must preserve the full-view bar ratios.');
  }
  const fullData = preview.getByRole('link', { name: 'More frameworks and methodology', exact: true });
  await expect(fullData).toHaveAttribute('href', '/webui/reference/benchmarks/');
  await expect(fullData).toBeVisible();
  await expect(preview.locator('details')).toHaveCount(0);
  const bootstrap = JSON.parse(await page.locator('#webui-data').textContent() ?? 'null') as {
    state: Record<string, unknown> & { previewMetrics: BenchmarkPreview[] };
  };
  assert.ok(bootstrap.state.installOptions, 'The install component needs its command model.');
  assert.equal(bootstrap.state.previewMetrics.length, 4);
  assert.ok(bootstrap.state.previewMetrics.every(metric => metric.rows.length === 3));
  assert.ok(bootstrap.state.previewMetrics.every(metric => metric.rows.every(row => !('values' in row))));
  assert.ok(Buffer.byteLength(JSON.stringify(bootstrap.state.previewMetrics)) < 4096,
    'hydrate only four compact three-case views below 4 KiB');
  assert.ok(!('benchmarkNotes' in bootstrap.state), 'static measurement notes need no bootstrap state');
  assert.ok(!('previewCaseCount' in bootstrap.state));
  assert.ok(!('previewTotalCases' in bootstrap.state));
  assert.ok(!('benchmarks' in bootstrap.state), 'The full explorer dataset must stay off the homepage.');
  const installBox = await install.boundingBox();
  const previewBox = await preview.boundingBox();
  assert.ok(installBox && previewBox && previewBox.y > installBox.y + installBox.height,
    'Mobile installation must precede the evidence, not sit beside it.');
  await fullData.focus();
  await expect(fullData).toHaveCSS('outline-style', 'solid');
  await page.keyboard.press('Enter');
  await expect(page).toHaveURL(`${server.origin}/webui/reference/benchmarks/`);
  const notes = page.locator('.benchmark-notes');
  await expect(notes).toContainText('Official campaign results for a 100-todo app');
  await expect(notes).toContainText('1 measured round (20 s)');
  await expect(notes).toContainText('variability is not established');
  await expect(notes).toContainText('Some samples were unavailable');
  await expect(notes).toContainText('not a complete raw-run archive');
});

test('install choices copy exact commands and support native keyboard selection', async t => {
  const server = await startWebsite();
  t.after(() => server.close());
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage({ viewport: { width: 320, height: 844 } });
  await page.route('https://**', route => route.abort());
  await page.addInitScript(() => {
    window.installTestCopies = [];
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText: async (command: string) => { window.installTestCopies.push(command); } },
    });
  });
  await page.goto(`${server.origin}/webui/`);
  const install = page.locator('install-command');
  const copy = install.getByRole('button', { name: 'Copy install command' });
  await expect(copy).toBeEnabled();
  for (const [label, command] of commands) {
    await install.getByText(label, { exact: true }).click();
    await expect(install.locator('code')).toHaveText(command);
    await copy.click();
    await expect(install.getByRole('status')).toHaveText('Copied. Run it in your project folder.');
  }
  assert.deepEqual(await page.evaluate(() => window.installTestCopies), commands.map(([, command]) => command));
  assert.equal(await page.evaluate(() => customElements.get('benchmark-explorer') !== undefined), false);
  await install.getByRole('radio', { name: 'npm', exact: true }).focus();
  await page.keyboard.press('ArrowRight');
  await expect(install.getByRole('radio', { name: 'pnpm', exact: true })).toBeChecked();
  await expect(install.getByRole('radio', { name: 'pnpm', exact: true })).toBeFocused();
  await expect(install.getByText('pnpm', { exact: true })).toHaveCSS('outline-style', 'solid');
  await expect(install.locator('code')).toHaveText(commands[1][1]);
});

test('clipboard denial and unavailable APIs show a manual-copy recovery instead of success', async t => {
  const server = await startWebsite();
  t.after(() => server.close());
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  for (const unavailable of [false, true]) {
    const page = await browser.newPage();
    await page.route('https://**', route => route.abort());
    await page.addInitScript(missing => {
      Object.defineProperty(navigator, 'clipboard', {
        configurable: true,
        value: missing ? undefined : {
          writeText: async () => { throw new DOMException('Denied in test', 'NotAllowedError'); },
        },
      });
    }, unavailable);
    const errors: string[] = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(`${server.origin}/webui/`);
    const install = page.locator('install-command');
    const copy = install.getByRole('button', { name: 'Copy install command' });
    await expect(copy).toBeEnabled();
    await copy.click();
    await expect(install.getByRole('status')).toHaveText('Copy failed. Select the command and copy it manually.');
    await expect(copy).toBeEnabled();
    await expect(install.locator('code')).toHaveText(commands[0][1]);
    assert.deepEqual(errors, []);
    await page.close();
  }
});

test('a completed clipboard request cannot claim a newly selected command was copied', async t => {
  const server = await startWebsite();
  t.after(() => server.close());
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage();
  await page.route('https://**', route => route.abort());
  await page.addInitScript(() => {
    window.installTestCopies = [];
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: {
        writeText: (command: string) => new Promise<void>(resolve => {
          window.installTestCopies.push(command);
          window.finishInstallTestCopy = resolve;
        }),
      },
    });
  });
  await page.goto(`${server.origin}/webui/`);
  const install = page.locator('install-command');
  const copy = install.getByRole('button', { name: 'Copy install command' });
  await expect(copy).toBeEnabled();
  await copy.click();
  await expect(copy).toBeDisabled();
  await install.getByText('pnpm', { exact: true }).click();
  await expect(install.locator('code')).toHaveText(commands[1][1]);
  await expect(copy).toBeDisabled();
  await page.evaluate(() => window.finishInstallTestCopy?.());
  await expect(copy).toBeEnabled();
  await expect(install.getByRole('status')).not.toContainText('Copied');
  assert.deepEqual(await page.evaluate(() => window.installTestCopies), [commands[0][1]]);
  await copy.click();
  await expect(copy).toBeDisabled();
  await page.evaluate(() => window.finishInstallTestCopy?.());
  await expect(install.getByRole('status')).toContainText('Copied');
  assert.deepEqual(await page.evaluate(() => window.installTestCopies), [commands[0][1], commands[1][1]]);
});

test('the approved entry layout keeps desktop action and chart visible without overflow', async t => {
  const server = await startWebsite();
  t.after(() => server.close());
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  for (const [width, height] of [[1440, 900], [1280, 800], [820, 1180], [390, 844], [320, 844]]) {
    const page = await browser.newPage({ viewport: { width, height }, reducedMotion: 'reduce' });
    await page.route('https://**', route => route.abort());
    await page.goto(`${server.origin}/webui/`);
    await expect(page.locator('install-command button')).toBeEnabled();
    const main = await page.locator('.main-content').evaluate(element => ({
      client: element.clientWidth, scroll: element.scrollWidth,
    }));
    assert.ok(main.scroll <= main.client + 1, `${width}px must not overflow horizontally.`);
    const install = await page.locator('install-command').boundingBox();
    const actions = await page.locator('.intro-actions').boundingBox();
    const preview = await page.locator('benchmark-preview').boundingBox();
    assert.ok(install && actions && preview);
    assert.ok(actions.y + actions.height <= install.y, 'Offer guided entry points before the install command.');
    if (width >= 1280) {
      assert.ok(install.y + install.height <= height, `${width}px: install must fit in the first viewport.`);
      assert.ok(preview.y + preview.height <= height, `${width}px: the benchmark card must fit.`);
      assert.ok(preview.x > install.x + install.width, 'Desktop proof must sit to the right of installation.');
    } else if (width <= 700) {
      assert.ok(preview.y > install.y + install.height, 'Mobile proof must follow the installation flow.');
    }
    const plot = await page.locator('benchmark-preview .plot').first().boundingBox();
    const scale = await page.locator('benchmark-preview .scale').boundingBox();
    assert.ok(plot && scale);
    assert.ok(Math.abs(plot.x - scale.x) <= 1, `${width}px: the zero label must align with the bar origin.`);
    assert.ok(Math.abs(plot.width - scale.width) <= 1, `${width}px: the scale must span the plotted domain.`);
    assert.ok(plot.width >= 120, `${width}px: stack chart labels instead of squeezing the bars.`);
    await page.close();
  }
  const source = fs.readFileSync(path.join(docsDist, 'index.html'), 'utf8');
  assert.ok(!source.includes('benchmark-explorer.css'), 'The homepage must not load the full explorer styling.');
});

test('entry commands, evidence and controls remain readable in both themes and forced colors', async t => {
  const server = await startWebsite();
  t.after(() => server.close());
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  for (const colorScheme of ['light', 'dark'] as const) {
    const page = await browser.newPage({ colorScheme });
    await page.route('https://**', route => route.abort());
    await page.goto(`${server.origin}/webui/`);
    const copy = page.locator('install-command button');
    await expect(copy).toBeEnabled();
    await expect(page.locator('benchmark-preview').getByRole('tab', { name: 'RPS', exact: true })).toBeEnabled();
    const labels = await page.locator('benchmark-preview').evaluate(element => {
      const root = element.shadowRoot;
      const paper = root?.querySelector('.preview');
      if (!root || !paper) throw new Error('Missing benchmark preview');
      const background = getComputedStyle(paper).backgroundColor;
      return [...root.querySelectorAll('h2, .metric-tabs button, .metric-title, .direction, th, .value, .scale, a')]
        .map(label => ({ name: label.textContent, color: getComputedStyle(label).color, background }));
    });
    const controls = await page.locator('install-command code, install-command button').evaluateAll(elements =>
      elements.map(element => {
        const paper = element instanceof HTMLButtonElement ? element : element.closest('.command-line');
        if (!paper) throw new Error('Missing command container');
        return {
          name: element.textContent,
          color: getComputedStyle(element).color,
          background: getComputedStyle(paper).backgroundColor,
        };
      }));
    for (const colors of [...labels, ...controls]) {
      assert.ok(contrastRatio(colors.color, colors.background) >= 4.5,
        `${colorScheme}: ${colors.name} needs at least 4.5:1 text contrast.`);
    }
    await page.emulateMedia({ forcedColors: 'active' });
    await expect(copy).not.toHaveCSS('border-top-color', 'rgba(0, 0, 0, 0)');
    const chart = await page.locator('benchmark-preview').evaluate(element => {
      const paper = element.shadowRoot?.querySelector('.preview');
      const bar = element.shadowRoot?.querySelector('.bar');
      if (!paper || !bar) throw new Error('Missing high-contrast chart');
      return { color: getComputedStyle(bar).fill, background: getComputedStyle(paper).backgroundColor };
    });
    assert.ok(contrastRatio(chart.color, chart.background) >= 3, 'Forced-color bars must remain visible.');
    await page.close();
  }
});
