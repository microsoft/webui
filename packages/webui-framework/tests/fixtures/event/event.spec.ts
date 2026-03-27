// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('event fixture', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/event/fixture.html');
    await page.waitForSelector('test-event');
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
