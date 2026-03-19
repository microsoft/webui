// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * End-to-end tests for the Contact Book Manager app.
 *
 * Tests SSR rendering, client-side navigation, and visual regression.
 * The app uses shadow DOM components (cb-*) with FAST-HTML templating.
 */

import { test, expect } from '@playwright/test';

// ── SSR Tests (direct page loads) ────────────────────────────────

test.describe('SSR pages', () => {
  test('dashboard renders with stats', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('cb-page-dashboard .page-title')).toHaveText('Dashboard');
    // Stat cards
    await expect(page.locator('cb-page-dashboard .stat-label').filter({ hasText: 'Total Contacts' })).toBeVisible();
    await expect(page.locator('cb-page-dashboard .stat-label').filter({ hasText: 'Favorites' })).toBeVisible();
    await expect(page.locator('cb-page-dashboard .stat-label').filter({ hasText: 'Groups' })).toBeVisible();
    // Recent contacts section
    await expect(page.locator('cb-page-dashboard .section-title')).toContainText('Recent Contacts');
    await expect(page.locator('cb-page-dashboard cb-contact-card').first()).toBeVisible();
  });

  test('contacts page renders contact list', async ({ page }) => {
    await page.goto('/contacts');
    await expect(page.locator('cb-page-contacts .page-title')).toHaveText('All Contacts');
    await expect(page.locator('cb-page-contacts cb-contact-card')).toHaveCount(15);
    // Spot-check known contacts
    await expect(page.getByText('Sarah Chen')).toBeVisible();
    await expect(page.getByText('Marcus Johnson')).toBeVisible();
  });

  test('favorites page renders favorite contacts', async ({ page }) => {
    await page.goto('/favorites');
    await expect(page.locator('cb-page-favorites .page-title')).toHaveText('Favorites');
    await expect(page.locator('cb-page-favorites cb-contact-card')).toHaveCount(5);
    await expect(page.getByText('Sarah Chen')).toBeVisible();
    await expect(page.getByText('Yuki Tanaka')).toBeVisible();
  });
});

// ── Client-side navigation tests ─────────────────────────────────

test.describe('client-side navigation', () => {
  test('navigate dashboard to contacts', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('cb-page-dashboard .page-title')).toHaveText('Dashboard');

    await page.locator('cb-sidebar').getByRole('link', { name: /All Contacts/ }).click();
    await expect(page).toHaveURL('/contacts');
    await expect(page.locator('cb-page-contacts .page-title')).toHaveText('All Contacts');
  });

  test('navigate contacts to favorites via sidebar', async ({ page }) => {
    await page.goto('/contacts');
    await expect(page.locator('cb-page-contacts .page-title')).toHaveText('All Contacts');

    await page.locator('cb-sidebar').getByRole('link', { name: /Favorites/ }).click();
    await expect(page).toHaveURL('/favorites');
    await expect(page.locator('cb-page-favorites .page-title')).toHaveText('Favorites');
  });

  test('navigate to group via sidebar', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('link', { name: 'Work' }).first().click();
    await expect(page).toHaveURL('/groups/work');
    await expect(page.locator('cb-page-group .page-title')).toContainText('Work');
  });

  test('navigate contacts to contact detail via click', async ({ page }) => {
    await page.goto('/contacts');
    await expect(page.locator('cb-page-contacts .page-title')).toHaveText('All Contacts');

    await page.locator('cb-contact-card').first().click();
    await expect(page).toHaveURL(/\/contacts\/\d+/);
    await expect(page.locator('cb-contact-detail')).toBeVisible();
  });

  test('navigate back from contact detail', async ({ page }) => {
    await page.goto('/contacts/1');
    await expect(page.locator('cb-contact-detail')).toBeVisible();

    await page.locator('cb-sidebar').getByRole('link', { name: /All Contacts/ }).click();
    await expect(page).toHaveURL('/contacts');
    await expect(page.locator('cb-page-contacts .page-title')).toHaveText('All Contacts');
  });

  test('no full page reload during navigation', async ({ page }) => {
    await page.goto('/');
    await page.evaluate(() => { (window as any).__testMarker = Date.now(); });

    await page.locator('cb-sidebar').getByRole('link', { name: /All Contacts/ }).click();
    await expect(page.locator('cb-page-contacts .page-title')).toHaveText('All Contacts');

    await page.locator('cb-sidebar').getByRole('link', { name: /Favorites/ }).click();
    await expect(page.locator('cb-page-favorites .page-title')).toHaveText('Favorites');

    const marker = await page.evaluate(() => (window as any).__testMarker);
    expect(marker).toBeGreaterThan(0);
  });
});

// ── Visual regression tests ──────────────────────────────────────

test.describe('visual regression', () => {
  test('dashboard screenshot', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('cb-page-dashboard .page-title')).toHaveText('Dashboard');
    await expect(page).toHaveScreenshot('dashboard.png', { maxDiffPixelRatio: 0.01 });
  });

  test('contacts page screenshot', async ({ page }) => {
    await page.goto('/contacts');
    await expect(page.locator('cb-page-contacts .page-title')).toHaveText('All Contacts');
    await expect(page).toHaveScreenshot('contacts.png', { maxDiffPixelRatio: 0.01 });
  });

  test('favorites page screenshot', async ({ page }) => {
    await page.goto('/favorites');
    await expect(page.locator('cb-page-favorites .page-title')).toHaveText('Favorites');
    await expect(page).toHaveScreenshot('favorites.png', { maxDiffPixelRatio: 0.01 });
  });

  test('group page screenshot', async ({ page }) => {
    await page.goto('/groups/work');
    await expect(page.locator('cb-page-group .page-title')).toContainText('Work');
    await expect(page).toHaveScreenshot('group-work.png', { maxDiffPixelRatio: 0.01 });
  });

  test('contact detail screenshot', async ({ page }) => {
    await page.goto('/contacts/1');
    await expect(page.locator('cb-contact-detail')).toBeVisible();
    await expect(page).toHaveScreenshot('contact-detail.png', { maxDiffPixelRatio: 0.01 });
  });
});
