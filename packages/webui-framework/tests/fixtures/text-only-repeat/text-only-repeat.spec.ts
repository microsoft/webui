// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('text-only repeat fixture', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/text-only-repeat/fixture.html');
    await page.waitForSelector('test-text-only-repeat');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-text-only-repeat');
      return el && (el as any).$ready === true;
    });
  });

  test('SSR renders only the active option label', async ({ page }) => {
    // After hydration, the label should show exactly "Relevance" — not duplicated
    const label = page.locator('test-text-only-repeat .label');
    await expect(label).toHaveText('Relevance');

    // Verify no duplication — text content should be exactly "Relevance"
    const text = await label.textContent();
    expect(text?.trim()).toBe('Relevance');
  });

  test('updating options does not duplicate the active label', async ({ page }) => {
    // Call onUpdate directly to avoid event wiring issues in this test
    await page.evaluate(() => {
      (document.querySelector('test-text-only-repeat') as any).onUpdate();
    });

    const label = page.locator('test-text-only-repeat .label');
    // Should show exactly "Trending" — not "RelevanceTrending"
    await expect(label).toHaveText('Trending');
  });

  test('only one active label is visible at a time', async ({ page }) => {
    // Count .active-label elements — should be exactly 1
    const count = await page.locator('test-text-only-repeat .active-label').count();
    expect(count).toBe(1);

    // After update, still exactly 1
    await page.evaluate(() => {
      (document.querySelector('test-text-only-repeat') as any).onUpdate();
    });
    const countAfter = await page.locator('test-text-only-repeat .active-label').count();
    expect(countAfter).toBe(1);
  });
});
