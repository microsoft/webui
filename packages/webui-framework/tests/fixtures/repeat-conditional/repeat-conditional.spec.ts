// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

for (const mode of ['light', 'shadow'] as const) {
test.describe(`repeat conditional fixture [${mode} DOM]`, () => {
  test.beforeEach(async ({ page }) => {
    const file = mode === 'light' ? 'fixture.html' : 'fixture-shadow.html';
    await page.goto(`/repeat-conditional/${file}`);
    await page.waitForSelector('test-repeat-conditional');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-repeat-conditional');
      return el && (el as any).$ready === true;
    });
  });

  test('expands conditional branches inside client repeat updates', async ({ page }) => {
    await page.locator('test-repeat-conditional .load').click();

    await expect(page.locator('test-repeat-conditional .current')).toHaveText('Shirts');
    await expect(page.locator('test-repeat-conditional .link')).toHaveText(['Headwear', 'Archived']);
    await expect(page.locator('test-repeat-conditional .link').nth(1)).toBeDisabled();

    const ifCount = await page.evaluate(() => {
      const host = document.querySelector('test-repeat-conditional');
      return (host?.shadowRoot ?? host)?.querySelectorAll('if').length ?? -1;
    });

    expect(ifCount).toBe(0);
  });

  test('hydrates SSR repeat conditionals with non-local marker ids', async ({ page }) => {
    await expect(page.locator('test-repeat-conditional .current')).toHaveText('Shirts');
    await expect(page.locator('test-repeat-conditional .link')).toHaveText(['Headwear', 'Archived']);
    await expect(page.locator('test-repeat-conditional .link').nth(1)).toBeDisabled();

    await page.locator('test-repeat-conditional .switch').click();

    await expect(page.locator('test-repeat-conditional .current')).toHaveText('Headwear');
    await expect(page.locator('test-repeat-conditional .link')).toHaveText(['Shirts', 'Archived']);
    await expect(page.locator('test-repeat-conditional .link').nth(1)).toBeEnabled();
    await expect(page.locator('test-repeat-conditional .link').first()).toHaveAttribute('data-href', '/search/shirts');
  });

  test('re-evaluates repeat conditionals and boolean attrs on subsequent updates', async ({ page }) => {
    await page.locator('test-repeat-conditional .load').click();
    await page.locator('test-repeat-conditional .switch').click();

    await expect(page.locator('test-repeat-conditional .current')).toHaveText('Headwear');
    await expect(page.locator('test-repeat-conditional .link')).toHaveText(['Shirts', 'Archived']);
    await expect(page.locator('test-repeat-conditional .link').nth(1)).toBeEnabled();
    await expect(page.locator('test-repeat-conditional .link').first()).toHaveAttribute('data-href', '/search/shirts');
  });
});
}
