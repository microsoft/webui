// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('sidebar repeat fixture', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/sidebar-repeat/fixture.html');
    await page.waitForSelector('test-sidebar-repeat');
  });

  async function expectActiveNav(page: import('@playwright/test').Page, nav: string): Promise<void> {
    const active = page.locator('test-sidebar-repeat [data-active]');
    await expect(active).toHaveCount(1);
    await expect(active).toHaveAttribute('data-nav', nav);
  }

  test('keeps SSR repeated anchors in the correct section when syncing groups', async ({ page }) => {
    const sections = page.locator('test-sidebar-repeat .nav-section');

    await expect(sections.nth(0).locator('.nav-item')).toHaveText([
      'Dashboard',
      'All Contacts',
      'Favorites',
    ]);
    await expect(sections.nth(1).locator('.nav-item-group')).toHaveText([
      'work',
      'family',
      'friends',
      'other',
    ]);

    await page.locator('test-sidebar-repeat').evaluate((el) => {
      (el as unknown as { syncGroups(): void }).syncGroups();
    });

    await expect(sections.nth(0).locator('.nav-item')).toHaveText([
      'Dashboard',
      'All Contacts',
      'Favorites',
    ]);
    await expect(sections.nth(1).locator('.nav-item-group')).toHaveText([
      'work',
      'family',
      'friends',
      'other',
    ]);
    await expect(page.locator('test-sidebar-repeat .nav-item-group')).toHaveCount(4);
  });

  test('updates active nav markers for top-level and repeated links', async ({ page }) => {
    await expectActiveNav(page, 'Dashboard');

    await page.locator('test-sidebar-repeat').evaluate((el) => {
      const host = el as unknown as { page: string; activeGroup: string };
      host.page = 'favorites';
      host.activeGroup = '';
    });
    await expectActiveNav(page, 'Favorites');

    await page.locator('test-sidebar-repeat').evaluate((el) => {
      const host = el as unknown as { page: string; activeGroup: string };
      host.page = 'group';
      host.activeGroup = 'work';
    });
    await expectActiveNav(page, 'work');

    await page.locator('test-sidebar-repeat').evaluate((el) => {
      const host = el as unknown as { page: string; activeGroup: string };
      host.page = 'contacts';
      host.activeGroup = '';
    });
    await expectActiveNav(page, 'All Contacts');
  });
});
