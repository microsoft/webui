// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

for (const mode of ['light', 'shadow'] as const) {
test.describe(`event fixture [${mode} DOM]`, () => {
  test.beforeEach(async ({ page }) => {
    const file = mode === 'light' ? 'fixture.html' : 'fixture-shadow.html';
    await page.goto(`/event/${file}`);
    await page.waitForSelector('test-event');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-event');
      return el && (el as any).$ready === true;
    });
  });

  test('renders the initial count', async ({ page }) => {
    await expect(page.locator('test-event .count')).toHaveText('0');
  });

  test('increments on click', async ({ page }) => {
    await page.locator('test-event .inc').click();
    await expect(page.locator('test-event .count')).toHaveText('1');
  });

  test('decrements after multiple increments', async ({ page }) => {
    await page.locator('test-event .inc').click();
    await page.locator('test-event .inc').click();
    await page.locator('test-event .dec').click();

    await expect(page.locator('test-event .count')).toHaveText('1');
  });

  test('resets to zero', async ({ page }) => {
    await page.locator('test-event .inc').click();
    await page.locator('test-event .inc').click();
    await page.locator('test-event .reset').click();

    await expect(page.locator('test-event .count')).toHaveText('0');
  });

  test('hydrates SSR event bindings with non-local marker ids', async ({ page }) => {
    await page.locator('test-event .inc').click();
    await page.locator('test-event .dec').click();
    await page.locator('test-event .reset').click();

    await expect(page.locator('test-event .count')).toHaveText('0');
  });
});
}
