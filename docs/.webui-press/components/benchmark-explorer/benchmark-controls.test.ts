// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { chromium, expect } from '@playwright/test';
import type { BrowserContext } from '@playwright/test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { docsDist as dist } from '../../../../packages/webui-press/tests/website-fixture.js';

const origin = 'http://benchmark.test';

async function serveBuiltSite(context: BrowserContext): Promise<void> {
  const contentTypes: Record<string, string> = {
    '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css',
    '.svg': 'image/svg+xml', '.json': 'application/json',
  };
  await context.route('**/*', async route => {
    const url = new URL(route.request().url());
    if (url.origin !== origin || !url.pathname.startsWith('/webui/')) {
      await route.abort();
      return;
    }
    let file = path.resolve(dist, decodeURIComponent(url.pathname.slice('/webui/'.length)));
    assert.ok(file === dist || file.startsWith(dist + path.sep), 'request escaped the built site');
    if (fs.existsSync(file) && fs.statSync(file).isDirectory()) file = path.join(file, 'index.html');
    if (!fs.existsSync(file)) {
      await route.fulfill({ status: 404, body: 'Not found' });
      return;
    }
    await route.fulfill({ path: file, contentType: contentTypes[path.extname(file)] });
  });
}

test('benchmark controls switch metrics with a compact mobile select and retain a disabled no-script fallback', async t => {
  assert.ok(fs.existsSync(path.join(dist, 'reference/benchmarks/index.html')), 'Build the docs before testing benchmark controls.');
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  const noScript = await browser.newContext({
    javaScriptEnabled: false, viewport: { width: 390, height: 900 },
  });
  await serveBuiltSite(noScript);
  const staticPage = await noScript.newPage();
  await staticPage.goto(`${origin}/webui/reference/benchmarks/`);
  const staticExplorer = staticPage.locator('benchmark-explorer');
  const staticSelect = staticExplorer.getByRole('combobox', { name: 'Measurement to plot' });
  await expect(staticSelect).toBeVisible();
  await expect(staticSelect).toBeDisabled();
  await expect(staticSelect).toHaveValue('jsHeapUsedBytes');
  await expect(staticSelect.locator('option')).toHaveCount(5);
  await expect(staticExplorer.locator('.benchmark-metrics')).toBeHidden();
  await expect(staticExplorer.locator('.benchmark-row')).toHaveCount(15);
  await expect(staticExplorer.locator('svg.benchmark-plot')).toHaveCount(15);
  await expect(staticExplorer.locator('.benchmark-figure-heading h3')).toContainText('JavaScript heap');
  await staticExplorer.locator('.benchmark-data-table > summary').click();
  await expect(staticExplorer.locator('table')).toBeVisible();
  await expect(staticExplorer.locator('tbody tr')).toHaveCount(15);
  await expect(staticExplorer.locator('tbody tr').first().locator('td')).toHaveCount(5);
  await staticPage.setViewportSize({ width: 1280, height: 900 });
  await expect(staticSelect).toBeHidden();
  await expect(staticExplorer.locator('.benchmark-metrics')).toBeVisible();
  for (const button of await staticExplorer.locator('.benchmark-metrics button').all()) {
    await expect(button).toBeDisabled();
  }
  await noScript.close();

  const context = await browser.newContext({ viewport: { width: 390, height: 900 } });
  await serveBuiltSite(context);
  const page = await context.newPage();
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(`${origin}/webui/reference/benchmarks/`);
  const explorer = page.locator('benchmark-explorer');
  const select = explorer.getByRole('combobox', { name: 'Measurement to plot' });
  await expect(select).toBeEnabled();
  await expect(select).toHaveValue('jsHeapUsedBytes');
  await expect(explorer.locator('.benchmark-metrics')).toBeHidden();
  await explorer.evaluate(element => element.scrollIntoView({ block: 'start' }));
  const toolbar = await explorer.locator('.benchmark-toolbar').boundingBox();
  assert.ok(toolbar && toolbar.height < 360, 'mobile controls must not consume most of the viewport');
  await expect(explorer.locator('svg.benchmark-plot').first()).toBeInViewport({ ratio: 1 });

  await select.selectOption('streamingRequestsPerSecond');
  await expect(explorer.locator('.benchmark-figure-heading h3')).toContainText('Progressive response');
  await expect(explorer.locator('.benchmark-direction')).toHaveText('Higher serves more requests');
  await expect(explorer.locator('.benchmark-row').first()).toHaveAttribute('data-case', 'webui-rust-actix');
  await expect(explorer.locator('.benchmark-row')).toHaveCount(15);

  await select.focus();
  await select.press('j');
  await expect(select).toHaveValue('jsHeapUsedBytes');
  await select.press('Tab');
  await page.keyboard.press('Shift+Tab');
  await expect(select).toBeFocused();
  await select.press('r');
  await expect(select).toHaveValue('rendererPrivateBytes');
  await expect(select).toBeFocused();
  await expect(explorer.locator('.benchmark-figure-heading h3')).toContainText('Renderer private bytes');

  await page.setViewportSize({ width: 1280, height: 900 });
  await expect(select).toBeHidden();
  await expect(explorer.locator('.benchmark-metrics')).toBeVisible();
  await expect(explorer.locator('[data-metric="rendererPrivateBytes"]')).toHaveAttribute('aria-pressed', 'true');
  await explorer.locator('[data-metric="largestContentfulPaintMs"]').click();
  await expect(explorer.locator('.benchmark-figure-heading h3')).toContainText('Largest contentful paint');
  await page.setViewportSize({ width: 390, height: 900 });
  await expect(select).toBeVisible();
  await expect(select).toHaveValue('largestContentfulPaintMs');
  assert.deepEqual(errors, []);
});
