// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('nav fixture', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/nav/fixture.html');
    await page.waitForSelector('test-nav');
  });

  test('preserves static anchor siblings when syncing repeated anchors', async ({ page }) => {
    await expect(page.locator('test-nav .primary .nav-link')).toHaveText([
      'Dashboard',
      'All Contacts',
      'Favorites',
    ]);
    await expect(page.locator('test-nav .groups .nav-link-group')).toHaveText([
      'work',
      'family',
      'friends',
      'other',
    ]);

    await page.locator('test-nav .sync').click();

    await expect(page.locator('test-nav .primary .nav-link')).toHaveText([
      'Dashboard',
      'All Contacts',
      'Favorites',
    ]);
    await expect(page.locator('test-nav .groups .nav-link-group')).toHaveText([
      'work',
      'family',
      'friends',
      'other',
    ]);
  });
});
