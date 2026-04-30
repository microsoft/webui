// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { test, expect } from '@playwright/test';

test.describe('SSR rendering', () => {
  test('renders calculator display', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('calc-app')).toBeVisible();
    await expect(page.locator('calc-display')).toBeVisible();
  });

  test('renders calculator buttons', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('calc-button[label="7"]')).toBeVisible();
    await expect(page.locator('calc-button[label="8"]')).toBeVisible();
    await expect(page.locator('calc-button[label="9"]')).toBeVisible();
    await expect(page.locator('calc-button[label="="]')).toBeVisible();
    await expect(page.locator('calc-button[label="AC"]')).toBeVisible();
  });

  test('renders in standard mode', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('calc-app[mode="standard"]')).toBeVisible();
    await expect(page.locator('calc-display[value="0"]')).toBeVisible();
  });
});

test.describe('visual regression', () => {
  test('calculator screenshot', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('calc-app')).toBeVisible();
    await expect(page).toHaveScreenshot('calculator.png', { maxDiffPixelRatio: 0.01 });
  });

  test('scientific mode screenshot', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('calc-app')).toBeVisible();
    // Wait for hydration — the mode-tab click handler needs JS
    await page.waitForFunction(() => customElements.get('calc-app') !== undefined);
    await page.getByRole('button', { name: 'Scientific' }).click();
    // Wait for mode to change
    await expect(page.locator('calc-app[mode="scientific"]')).toBeVisible({ timeout: 5000 });
    await expect(page).toHaveScreenshot('calculator-scientific.png', { maxDiffPixelRatio: 0.01 });
  });
});
