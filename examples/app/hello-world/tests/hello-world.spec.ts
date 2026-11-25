// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { test, expect } from '@playwright/test';

test.describe('SSR rendering', () => {
  test('renders raw HTML description', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('b')).toContainText('WebUI');
    await expect(page.locator('i')).toContainText('Rust');
  });

  test('renders contact card when condition is true', async ({ page }) => {
    await page.goto('/');
    await expect(page.getByText('Mohamed Mansour')).toBeVisible();
  });
});

test.describe('visual regression', () => {
  test('home page screenshot', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('h1')).toBeVisible();
    await expect(page).toHaveScreenshot('home-page.png', { maxDiffPixelRatio: 0.01 });
  });
});
