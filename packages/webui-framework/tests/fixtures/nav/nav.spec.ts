// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

for (const mode of ['light', 'shadow'] as const) {
test.describe(`nav fixture [${mode} DOM]`, () => {
  test.beforeEach(async ({ page }) => {
    const file = mode === 'light' ? 'fixture.html' : 'fixture-shadow.html';
    await page.goto(`/nav/${file}`);
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
}
