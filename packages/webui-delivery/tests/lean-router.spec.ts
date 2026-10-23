// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

interface TestRouter {
  gc(): void;
  activeComponent: string;
}

test.beforeEach(async ({ page }) => {
  page.on('pageerror', error => console.error(error));
  page.on('console', message => {
    if (message.type() === 'error') console.error(message.text());
  });
  await page.route('**/index.js', async route => {
    const response = await route.fetch({
      url: new URL('/lean-entry.js', route.request().url()).href,
    });
    await route.fulfill({ response });
  });
  await page.goto('/alpha');
  await page.waitForFunction(() =>
    (window as unknown as { __testRouter?: TestRouter }).__testRouter?.activeComponent === 'page-alpha',
  );
});

test('JSON-only navigation hydrates destination UI without a document reload', async ({ page }) => {
  await page.evaluate(() => { document.body.dataset.instance = 'same-document'; });
  const response = page.waitForResponse(response =>
    new URL(response.url()).pathname === '/beta' &&
    response.request().headers().accept === 'application/json',
  );
  await page.locator('nav a[href="/beta"]').click();
  expect((await response).ok()).toBe(true);
  await expect(page.locator('h2')).toContainText('Beta Page');
  await expect(page.locator('body')).toHaveAttribute('data-instance', 'same-document');
});

test('template collection preserves active bindings and allows later navigation', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  const released = await page.evaluate(() => {
    const router = (window as unknown as { __testRouter: TestRouter }).__testRouter;
    router.gc();
    return {
      inventory: window.__webui?.inventory,
      epoch: window.__webui?.inventoryEpoch,
    };
  });
  expect(released.inventory).toBe('');
  expect(released.epoch).toBe(1);
  await expect(page.locator('h2')).toContainText('Alpha Page');

  await page.locator('nav a[href="/beta"]').click();
  await expect(page.locator('h2')).toContainText('Beta Page');
  await page.goBack();
  await expect(page.locator('h2')).toContainText('Alpha Page');
  expect(errors).toEqual([]);
});
