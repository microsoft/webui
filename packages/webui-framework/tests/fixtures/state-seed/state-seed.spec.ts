// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

for (const mode of ['light', 'shadow'] as const) {
test.describe(`state-seed fixture [${mode} DOM]`, () => {
  test.beforeEach(async ({ page }) => {
    const file = mode === 'light' ? 'fixture.html' : 'fixture-shadow.html';
    await page.goto(`/state-seed/${file}`);
    await page.waitForSelector('test-state-seed');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-state-seed');
      return el && (el as any).$ready === true;
    });
  });

  test('reconstructs observable state from SSR DOM', async ({ page }) => {
    const state = await page.evaluate(() => {
      const host = document.querySelector('test-state-seed') as {
        title?: string;
        page?: string;
        groups?: string[];
        navCategories?: Array<{ handle: string; title: string; activeClass: string }>;
      } | null;

      return {
        title: host?.title ?? '',
        page: host?.page ?? '',
        groups: host?.groups ?? [],
        navCategories: host?.navCategories ?? [],
      };
    });

    expect(state).toEqual({
      title: 'SSR Title',
      page: 'dashboard',
      groups: ['work', 'family'],
      navCategories: [
        { handle: 'featured', title: 'Featured', activeClass: 'active' },
        { handle: 'sale', title: 'Sale', activeClass: '' },
      ],
    });
  });

  test('preserves reconstructed state on first updates', async ({ page }) => {
    await page.locator('test-state-seed .add-group').click();
    await page.locator('test-state-seed .add-category').click();

    await expect(page.locator('test-state-seed-shell .group-link')).toHaveText([
      'work',
      'family',
      'travel',
    ]);

    await expect(page.locator('test-state-seed-shell .category-link')).toHaveText([
      'Featured',
      'Sale',
      'Travel',
    ]);

    await expect(page.locator('test-state-seed-shell .category-link').first()).toHaveClass(/active/);
  });
});
}
