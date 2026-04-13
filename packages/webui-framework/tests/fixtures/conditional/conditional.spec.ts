// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('conditional fixture', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/conditional/fixture.html');
    await page.waitForSelector('test-conditional');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-conditional');
      return el && (el as any).$ready === true;
    });
  });

  test('renders the SSR conditional body', async ({ page }) => {
    await expect(page.locator('test-conditional .details')).toHaveText('Details');
  });

  test('toggles the conditional body on click', async ({ page }) => {
    await page.locator('test-conditional .toggle').click();
    await expect(page.locator('test-conditional .details')).toHaveCount(0);

    await page.locator('test-conditional .toggle').click();
    await expect(page.locator('test-conditional .details')).toHaveText('Details');
  });

  test('toggles the client-created conditional body on click', async ({ page }) => {
    await expect(page.locator('test-conditional-client .details')).toHaveText('Details');

    await page.locator('test-conditional-client .toggle').click();
    await expect(page.locator('test-conditional-client .details')).toHaveCount(0);

    await page.locator('test-conditional-client .toggle').click();
    await expect(page.locator('test-conditional-client .details')).toHaveText('Details');
  });

  test('toggles boolean attributes reactively', async ({ page }) => {
    await page.evaluate(() => {
      const host = document.querySelector('test-conditional') as { busy: boolean } | null;
      if (host) {
        host.busy = true;
      }
    });

    await expect(page.locator('test-conditional .toggle')).toBeDisabled();

    await page.evaluate(() => {
      const host = document.querySelector('test-conditional') as { busy: boolean } | null;
      if (host) {
        host.busy = false;
      }
    });

    await expect(page.locator('test-conditional .toggle')).toBeEnabled();
  });

  test('preserves text state when the same path also drives a conditional', async ({ page }) => {
    await page.evaluate(() => {
      const host = document.querySelector('test-conditional') as { busy: boolean } | null;
      if (host) {
        host.busy = true;
      }
    });

    await expect(page.locator('test-conditional .details')).toHaveText('Details');
    await expect(page.locator('test-conditional .toggle')).toBeDisabled();
  });

  test('negation simulates else branch — shows alternate when condition is false', async ({ page }) => {
    // !open is hidden when open=true
    await expect(page.locator('test-conditional .negated')).toHaveCount(0);

    await page.locator('test-conditional .toggle').click();
    // Now open=false, !open shows, details hides
    await expect(page.locator('test-conditional .details')).toHaveCount(0);
    await expect(page.locator('test-conditional .negated')).toHaveText('Negated visible');

    await page.locator('test-conditional .toggle').click();
    await expect(page.locator('test-conditional .details')).toHaveText('Details');
    await expect(page.locator('test-conditional .negated')).toHaveCount(0);
  });

  test('compound && condition requires both operands', async ({ page }) => {
    await expect(page.locator('test-conditional .compound-and')).toHaveText('And visible');

    await page.evaluate(() => {
      (document.querySelector('test-conditional') as any).busy = true;
    });
    await expect(page.locator('test-conditional .compound-and')).toHaveCount(0);
  });

  test('compound || condition requires at least one operand', async ({ page }) => {
    await expect(page.locator('test-conditional .compound-or')).toHaveText('Or visible');

    await page.locator('test-conditional .toggle').click();
    // open=false, busy=false → both false → hidden
    await expect(page.locator('test-conditional .compound-or')).toHaveCount(0);
  });

  test('comparison operator > evaluates numeric values', async ({ page }) => {
    await expect(page.locator('test-conditional .gt-zero')).toHaveText('Positive');

    await page.evaluate(() => {
      (document.querySelector('test-conditional') as any).count = 0;
    });
    await expect(page.locator('test-conditional .gt-zero')).toHaveCount(0);
  });
});
