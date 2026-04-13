// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('root-event fixture', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/root-event/fixture.html');
    await page.waitForSelector('test-root-event');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-root-event');
      return el && (el as any).$ready === true;
    });
  });

  test('root @click fires when clicking a child button', async ({ page }) => {
    await page.locator('test-root-event .action').click();
    await expect(page.locator('test-root-event .total')).toHaveText('1');
  });

  test('root @click fires for different child elements', async ({ page }) => {
    await page.locator('test-root-event .action').click();
    await page.locator('test-root-event .other').click();
    await expect(page.locator('test-root-event .total')).toHaveText('2');
  });

  test('root handler receives the event object with composedPath', async ({ page }) => {
    await page.locator('test-root-event .action').click();

    const action = await page.evaluate(() => {
      const el = document.querySelector('test-root-event') as any;
      return el?.lastAction;
    });
    expect(action).toBe('ping');
  });

  test('root handler distinguishes data-action from composedPath', async ({ page }) => {
    await page.locator('test-root-event .other').click();

    const action = await page.evaluate(() => {
      const el = document.querySelector('test-root-event') as any;
      return el?.lastAction;
    });
    expect(action).toBe('pong');
  });
});
