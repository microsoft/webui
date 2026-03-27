// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('list fixture', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/list/fixture.html');
    await page.waitForSelector('test-list');
    await expect(page.locator('test-list-item .title')).toHaveCount(2);
  });

  test('renders SSR repeat content and nested child conditionals', async ({ page }) => {
    await expect(page.locator('test-list-item .title')).toHaveText(['Alpha', 'Beta']);
    await expect(page.locator('test-list-item .done')).toHaveText(['Done']);
  });

  test('adds nested children through repeat reconciliation', async ({ page }) => {
    await page.locator('test-list .add').click();

    await expect(page.locator('test-list-item .title')).toHaveText(['Alpha', 'Beta', 'Item 3']);
    await expect(page.locator('test-list-item .done')).toHaveText(['Done', 'Done']);
  });

  test('preserves keyed nodes when reversing the collection', async ({ page }) => {
    const initial = await page.evaluate(() => {
      const host = document.querySelector('test-list');
      const root = host?.shadowRoot;
      const item = root?.querySelector('test-list-item[item-id="2"]');
      const win = window as unknown as { __preservedNode?: Element | null };
      win.__preservedNode = item;
      return item instanceof HTMLElement;
    });

    expect(initial).toBe(true);

    await page.locator('test-list .reverse').click();

    const preserved = await page.evaluate(() => {
      const host = document.querySelector('test-list');
      const root = host?.shadowRoot;
      const win = window as unknown as { __preservedNode?: Element | null };
      return win.__preservedNode === root?.querySelector('test-list-item[item-id="2"]');
    });

    expect(preserved).toBe(true);
    await expect(page.locator('test-list-item .title')).toHaveText(['Beta', 'Alpha']);
  });

  test('clears all repeated children', async ({ page }) => {
    await page.locator('test-list .clear').click();
    await expect(page.locator('test-list-item')).toHaveCount(0);
  });
});
