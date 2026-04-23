// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('split repeat fixture', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/split-repeat/fixture.html');
    await page.waitForSelector('test-split-repeat');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-split-repeat');
      return el && (el as any).$ready === true;
    });
  });

  test('keeps multiple SSR-hydrated repeats in their own containers', async ({ page }) => {
    await expect(page.locator('test-split-repeat .primary-item')).toHaveText(['Seed Alpha', 'Seed Beta']);
    await expect(page.locator('test-split-repeat .secondary-item')).toHaveText(['Seed One', 'Seed Two']);
    await expect(page.locator('test-split-repeat .primary .secondary-item')).toHaveCount(0);
    await expect(page.locator('test-split-repeat .secondary .primary-item')).toHaveCount(0);

    await page.locator('test-split-repeat .load').click();

    await expect(page.locator('test-split-repeat .primary-item')).toHaveText(['Alpha', 'Beta']);
    await expect(page.locator('test-split-repeat .secondary-item')).toHaveText(['One', 'Two', 'Three']);
    await expect(page.locator('test-split-repeat .primary .secondary-item')).toHaveCount(0);
    await expect(page.locator('test-split-repeat .secondary .primary-item')).toHaveCount(0);
  });

  test('keeps multiple client-created repeats in their own containers', async ({ page }) => {
    await page.locator('test-split-repeat .load').click();

    await expect(page.locator('test-split-repeat .primary-item')).toHaveText(['Alpha', 'Beta']);
    await expect(page.locator('test-split-repeat .secondary-item')).toHaveText(['One', 'Two', 'Three']);
    await expect(page.locator('test-split-repeat .primary .secondary-item')).toHaveCount(0);
    await expect(page.locator('test-split-repeat .secondary .primary-item')).toHaveCount(0);

    const compiledMarkers = await page.evaluate(() => {
      const meta = window.__webui?.templates?.['test-split-repeat'];
      const rootHtml = meta?.h ?? '';
      const blockHtml = (meta?.b ?? []).map((block) => block.h).join('');
      return {
        hasTextMarker: rootHtml.includes('<!--t:') || blockHtml.includes('<!--t:'),
        hasRepeatMarker: rootHtml.includes('<!--r:') || blockHtml.includes('<!--r:'),
        hasBindingAttr: rootHtml.includes('data-w-') || blockHtml.includes('data-w-'),
        hasEventAttr: rootHtml.includes('data-ev') || blockHtml.includes('data-ev'),
      };
    });

    expect(compiledMarkers).toEqual({
      hasTextMarker: false,
      hasRepeatMarker: false,
      hasBindingAttr: false,
      hasEventAttr: false,
    });
  });
});
