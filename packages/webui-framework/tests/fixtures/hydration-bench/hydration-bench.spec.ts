// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

interface HydrationTotals {
  totalMs: number;
  count: number;
}

test.describe('hydration bench fixture', () => {
  test('measures SSR hydration cost per component', async ({ page }) => {
    await page.goto('/hydration-bench/fixture.html');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-hydration-wide');
      return el && (el as unknown as { $ready?: boolean }).$ready === true;
    });

    const totals = await page.evaluate(
      () => (window as unknown as Record<string, Record<string, HydrationTotals>>).__hydrationBench,
    );

    console.log('\n=== WebUI Framework Hydration Benchmark ===');
    for (const [tag, entry] of Object.entries(totals)) {
      const per = entry.totalMs / entry.count;
      console.log(`${tag}: ${entry.count} instances, ${entry.totalMs.toFixed(2)}ms total, ${per.toFixed(4)}ms/instance`);
    }
    console.log('===========================================\n');

    // Hardware-dependent, so assert shape rather than absolute timings.
    expect(totals['test-hydration-wide'].count).toBe(150);
    expect(totals['test-hydration-deep'].count).toBe(150);
    expect(totals['test-hydration-wide'].totalMs).toBeGreaterThan(0);

    // Hydration must actually have wired the bindings.
    await expect(page.locator('test-hydration-wide').first().locator('.w0')).toHaveText('v0');
    await expect(page.locator('test-hydration-deep').first().locator('.d0')).toHaveText('v0');
  });

  test('keeps hydrated bindings reactive', async ({ page }) => {
    await page.goto('/hydration-bench/fixture.html');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-hydration-wide');
      return el && (el as unknown as { $ready?: boolean }).$ready === true;
    });

    await page.evaluate(() => {
      for (const el of document.querySelectorAll('test-hydration-wide')) {
        (el as unknown as { p0: string }).p0 = 'updated';
      }
    });

    await expect(page.locator('test-hydration-wide').first().locator('.w0')).toHaveText('updated');
    await expect(page.locator('test-hydration-wide').first().locator('.c0')).toHaveText('c0');
  });
});
