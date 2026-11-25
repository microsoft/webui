// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('composed root event — custom event pierces shadow DOM', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/composed-root-event/fixture.html');
    await page.waitForSelector('test-grandparent');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-grandparent');
      return el && (el as any).$ready === true;
    });
  });

  test('multiple clicks update correctly', async ({ page }) => {
    await page.locator('test-grandparent test-child[item-id="alpha"] .select-btn').click();
    await expect(page.locator('test-grandparent .result')).toHaveText('alpha');

    await page.locator('test-grandparent test-child[item-id="beta"] .select-btn').click();
    await expect(page.locator('test-grandparent .result')).toHaveText('beta');
  });

});
