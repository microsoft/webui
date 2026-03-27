// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('counter fixture', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/counter/fixture.html');
    await page.waitForSelector('test-counter');
  });

  test('renders SSR content', async ({ page }) => {
    await expect(page.locator('test-counter .label')).toHaveText('Clicks');
    await expect(page.locator('test-counter .count')).toHaveText('0');
    await expect(page.locator('test-counter .doubled')).toHaveText('0');
  });

  test('updates count through click handlers', async ({ page }) => {
    await page.locator('test-counter .inc').click();
    await page.locator('test-counter .inc').click();
    await page.locator('test-counter .dec').click();

    await expect(page.locator('test-counter .count')).toHaveText('1');
  });

  test('recomputes @volatile getters reactively', async ({ page }) => {
    await page.locator('test-counter .inc').click();
    await page.locator('test-counter .inc').click();

    await expect(page.locator('test-counter .doubled')).toHaveText('4');
  });

  test('updates @attr labels reactively', async ({ page }) => {
    await page.evaluate(() => {
      document.querySelector('test-counter')?.setAttribute('label', 'Count');
    });

    await expect(page.locator('test-counter .label')).toHaveText('Count');
  });
});
