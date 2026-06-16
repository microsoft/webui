// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test('streams WebUI-rendered chunks from a service worker', async ({ page }) => {
  const consoleErrors: string[] = [];
  const requestedUrls: string[] = [];
  page.on('console', (message) => {
    if (message.type() === 'error') {
      consoleErrors.push(message.text());
    }
  });
  page.on('request', (request) => {
    requestedUrls.push(request.url());
  });

  await page.goto('/').catch(() => {
    // The bootstrap page reloads once the service worker takes control.
  });
  await page.waitForFunction(() => navigator.serviceWorker.controller !== null);

  await expect(page.getByRole('heading', { name: 'WebUI rendered in a service worker' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Serverless HTML without an app server' })).toBeVisible();
  await expect(page.getByText('Chunks render as async state resolves')).toBeVisible();
  await expect(page.getByText('Streaming timeline')).toBeVisible();
  await expect(page.locator('.error-card')).toHaveCount(0);

  const headings = await page.locator('.card h1, .card h2').allTextContents();
  const shell = headings.indexOf('WebUI rendered in a service worker');
  const metrics = headings.indexOf('Chunks render as async state resolves');
  const hero = headings.indexOf('Serverless HTML without an app server');
  const activity = headings.indexOf('Streaming timeline');

  await expect(page.locator('.stream-chunk')).toHaveCount(4);
  const chunkOrder = await page.locator('.stream-chunk').evaluateAll((nodes) =>
    nodes.map((node) => node.getAttribute('data-chunk')),
  );
  expect(chunkOrder).toEqual(['shell', 'metrics', 'hero', 'activity']);

  expect(shell).toBeGreaterThan(-1);
  expect(metrics).toBeGreaterThan(shell);
  expect(hero).toBeGreaterThan(metrics);
  expect(activity).toBeGreaterThan(hero);

  await expect(page.locator('style')).toHaveCount(5);
  const themeCss = await page.locator('style').first().evaluate((node) => node.textContent ?? '');
  expect(themeCss).toContain('--color-brand-primary: #0078d4;');
  expect(requestedUrls.some((url) => url.endsWith('/styles.css'))).toBe(false);
  expect(consoleErrors).toEqual([]);
});
