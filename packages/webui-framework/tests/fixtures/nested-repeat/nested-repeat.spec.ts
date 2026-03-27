// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('nested repeat fixture', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/nested-repeat/fixture.html');
    await page.waitForSelector('test-nested-repeat');
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
});
