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

  test('removes direct event listeners when repeat items are cleared', async ({ page }) => {
    await page.addInitScript(() => {
      const originalRemove = EventTarget.prototype.removeEventListener;
      const counts = { blur: 0 };
      EventTarget.prototype.removeEventListener = function patchedRemoveEventListener(
        this: EventTarget,
        type: string,
        listener: EventListenerOrEventListenerObject,
        options?: boolean | EventListenerOptions,
      ): void {
        if (type === 'blur') counts.blur += 1;
        return originalRemove.call(this, type, listener, options);
      };
      (window as unknown as Record<string, unknown>).__repeatDirectRemovals = counts;
    });

    await page.goto('/repeat-conditional/fixture.html');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-repeat-conditional');
      return el && (el as any).$ready === true;
    });

    await expect(page.locator('test-repeat-conditional .link')).toHaveCount(2);
    await page.locator('test-repeat-conditional .clear').click();
    await expect(page.locator('test-repeat-conditional .link')).toHaveCount(0);
    await page.waitForFunction(() =>
      ((window as unknown as Record<string, { blur: number }>).__repeatDirectRemovals).blur >= 2,
    );
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
