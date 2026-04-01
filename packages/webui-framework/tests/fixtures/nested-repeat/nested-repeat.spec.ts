// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

for (const mode of ['light', 'shadow'] as const) {
test.describe(`nested repeat fixture [${mode} DOM]`, () => {
  test.beforeEach(async ({ page }) => {
    const file = mode === 'light' ? 'fixture.html' : 'fixture-shadow.html';
    await page.goto(`/nested-repeat/${file}`);
    await page.waitForSelector('test-nested-repeat');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-nested-repeat');
      return el && (el as any).$ready === true;
    });
  });

  test('resolves outer scope values inside nested repeat items', async ({ page }) => {
    await page.locator('test-nested-repeat .load').click();

    await expect(page.locator('test-nested-repeat h2')).toHaveText(['Color', 'Size']);
    await expect(page.locator('test-nested-repeat .value')).toHaveText(['Black', 'Blue', 'S', 'M']);
    await expect(page.locator('test-nested-repeat .value').nth(1)).toBeDisabled();

    const groups = await page.locator('test-nested-repeat .value').evaluateAll((elements) => {
      return elements.map((element) => element.getAttribute('data-group'));
    });

    expect(groups).toEqual(['Color', 'Color', 'Size', 'Size']);
  });

  test('updating groups with new objects does not duplicate inner items', async ({ page }) => {
    await page.locator('test-nested-repeat .load').click();
    await expect(page.locator('test-nested-repeat .value')).toHaveCount(4);

    // Re-set groups with new objects (same data) — triggers nested reconciliation
    await page.evaluate(() => {
      (document.querySelector('test-nested-repeat') as any).updateGroups();
    });

    // Must still have exactly 4 inner items, not 8
    await expect(page.locator('test-nested-repeat .value')).toHaveCount(4);
    await expect(page.locator('test-nested-repeat .value')).toHaveText(['Black', 'Blue', 'S', 'M']);
  });

  test('updating groups multiple times does not accumulate duplicates', async ({ page }) => {
    await page.locator('test-nested-repeat .load').click();
    await expect(page.locator('test-nested-repeat .value')).toHaveCount(4);

    for (let i = 0; i < 5; i++) {
      await page.evaluate(() => {
        (document.querySelector('test-nested-repeat') as any).updateGroups();
      });
    }

    await expect(page.locator('test-nested-repeat .value')).toHaveCount(4);
    await expect(page.locator('test-nested-repeat .value')).toHaveText(['Black', 'Blue', 'S', 'M']);

    const groups = await page.locator('test-nested-repeat .value').evaluateAll((elements) => {
      return elements.map((element) => element.getAttribute('data-group'));
    });
    expect(groups).toEqual(['Color', 'Color', 'Size', 'Size']);
  });

  test('growing an inner list does not duplicate existing items', async ({ page }) => {
    await page.locator('test-nested-repeat .load').click();
    await expect(page.locator('test-nested-repeat .value')).toHaveCount(4);

    await page.evaluate(() => {
      (document.querySelector('test-nested-repeat') as any).growFirstGroup();
    });

    await expect(page.locator('test-nested-repeat .value')).toHaveCount(5);
    await expect(page.locator('test-nested-repeat .value')).toHaveText([
      'Black', 'Blue', 'Red', 'S', 'M',
    ]);
  });

  test('shrinking an inner list removes items correctly', async ({ page }) => {
    await page.locator('test-nested-repeat .load').click();
    await expect(page.locator('test-nested-repeat .value')).toHaveCount(4);

    await page.evaluate(() => {
      (document.querySelector('test-nested-repeat') as any).shrinkFirstGroup();
    });

    await expect(page.locator('test-nested-repeat .value')).toHaveCount(3);
    await expect(page.locator('test-nested-repeat .value')).toHaveText(['Blue', 'S', 'M']);
  });
});
}
