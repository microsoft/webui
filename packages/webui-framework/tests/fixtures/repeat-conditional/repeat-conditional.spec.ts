// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('repeat conditional fixture', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/repeat-conditional/fixture.html');
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

  test('treats missing top-level and repeat-item paths as falsy operands', async ({ page }) => {
    const ssr = page.locator('test-repeat-conditional');
    await expect(ssr.locator('.missing-top-positive')).toHaveCount(0);
    await expect(ssr.locator('.missing-top-negated')).toHaveText('missing top-level path is falsy');
    await expect(ssr.locator('.missing-item-positive')).toHaveCount(0);
    await expect(ssr.locator('.missing-item-negated')).toHaveText(['Shirts', 'Headwear', 'Archived']);

    await page.evaluate(() => {
      const client = document.createElement('test-repeat-conditional');
      client.id = 'client-created-missing-paths';
      document.body.appendChild(client);
    });
    await page.waitForFunction(() => {
      const client = document.querySelector('#client-created-missing-paths');
      return client && (client as any).$ready === true;
    });

    const client = page.locator('#client-created-missing-paths');
    await expect(client.locator('.missing-top-positive')).toHaveCount(0);
    await expect(client.locator('.missing-top-negated')).toHaveText('missing top-level path is falsy');
    await expect(client.locator('.missing-item-positive')).toHaveCount(0);
    await expect(client.locator('.missing-item-negated')).toHaveText(['Shirts', 'Headwear', 'Archived']);

    await client.locator('.switch').click();
    await expect(client.locator('.missing-item-positive')).toHaveCount(0);
    await expect(client.locator('.missing-item-negated')).toHaveText(['Shirts', 'Headwear', 'Archived']);
  });

  test('re-evaluates repeat conditionals and boolean attrs on subsequent updates', async ({ page }) => {
    await page.locator('test-repeat-conditional .load').click();
    await page.locator('test-repeat-conditional .switch').click();

    await expect(page.locator('test-repeat-conditional .current')).toHaveText('Headwear');
    await expect(page.locator('test-repeat-conditional .link')).toHaveText(['Shirts', 'Archived']);
    await expect(page.locator('test-repeat-conditional .link').nth(1)).toBeEnabled();
    await expect(page.locator('test-repeat-conditional .link').first()).toHaveAttribute('data-href', '/search/shirts');
  });

  test('rewires repeat-scoped event handlers after structural updates', async ({ page }) => {
    await page.locator('test-repeat-conditional .switch').click();
    await page.locator('test-repeat-conditional .link').first().click();

    await expect(page.locator('test-repeat-conditional .selected-title')).toHaveText('Shirts');
  });
});
