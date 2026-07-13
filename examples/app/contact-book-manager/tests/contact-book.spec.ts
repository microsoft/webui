// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * End-to-end tests for the Contact Book Manager app.
 *
 * Tests SSR rendering, client-side navigation, and visual regression.
 * The app uses shadow DOM components (cb-*) with WebUI Framework templating.
 */

import { test, expect, type Page } from '@playwright/test';

function bootstrapStateFromHtml(html: string): Record<string, unknown> {
  const match = html.match(
    /<script[^>]+id=["']webui-data["'][^>]*>(.*?)<\/script>/s,
  );
  if (!match?.[1]) throw new Error('#webui-data bootstrap block missing');
  return (JSON.parse(match[1]) as { state?: Record<string, unknown> }).state ?? {};
}

async function expectSidebarGroupsStable(page: Page): Promise<void> {
  await expect(page.locator('cb-sidebar [data-nav="Dashboard"]')).toHaveCount(1);
  await expect(page.locator('cb-sidebar [data-nav="All Contacts"]')).toHaveCount(1);
  await expect(page.locator('cb-sidebar [data-nav="Favorites"]')).toHaveCount(1);
  await expect(page.locator('cb-sidebar .nav-item-group')).toHaveCount(4);
  await expect(page.locator('cb-sidebar .nav-item-group .nav-label')).toHaveText([
    'Work',
    'Family',
    'Friends',
    'Other',
  ]);
}

async function expectActiveSidebarNav(page: Page, nav: string): Promise<void> {
  const active = page.locator('cb-sidebar [data-active]');
  await expect(active).toHaveCount(1);
  await expect(active).toHaveAttribute('data-nav', nav);
}

async function expectContactDetailFieldsStable(page: Page): Promise<void> {
  await expect(page.locator('cb-contact-detail .detail-field').filter({ hasText: 'Address' }))
    .toContainText('123 Innovation Dr, Seattle, WA 98101');
  await expect(page.locator('cb-contact-detail .detail-field').filter({ hasText: 'Notes' }))
    .toContainText('Met at the tech conference in Seattle');
}

// ── SSR Tests (direct page loads) ────────────────────────────────

test.describe('SSR pages', () => {
  test('dashboard renders with stats', async ({ page }) => {
    const response = await page.goto('/');
    if (!response) throw new Error('dashboard navigation returned no response');
    const bootstrapState = bootstrapStateFromHtml(await response.text());
    expect(Object.keys(bootstrapState).sort()).toEqual(['totalFavorites']);
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

// ── Navigation tests ──────────────────────────────────────────────

test.describe('client-side navigation', () => {
  test('HTML-only components use dormant static hosts and soft navigation', async ({ page }) => {
    await page.goto('/contacts');

    await expect(page.locator('cb-page-contacts cb-contact-card')).toHaveCount(15);
    await expectSidebarGroupsStable(page);

    const autoDefined = await page.evaluate((tags) => {
      const results: boolean[] = [];
      for (let i = 0; i < tags.length; i++) {
        results.push(customElements.get(tags[i] ?? '') !== undefined);
      }
      return results;
    }, ['cb-sidebar', 'cb-page-contacts', 'cb-contact-card']);

    expect(autoDefined).toEqual([true, true, true]);

    await page.evaluate(() => {
      (window as Window & { navigationSentinel?: boolean }).navigationSentinel = true;
    });
    await page.locator('cb-sidebar').getByRole('link', { name: 'Work' }).click();
    await expect(page).toHaveURL('/groups/Work');
    await expect(page.locator('cb-page-group .page-title')).toContainText('Work');
    await expectActiveSidebarNav(page, 'Work');
    expect(await page.evaluate(
      () => (window as Window & { navigationSentinel?: boolean }).navigationSentinel,
    )).toBe(true);
  });

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

  test('sidebar groups do not duplicate across navigation', async ({ page }) => {
    await page.goto('/');
    await expectSidebarGroupsStable(page);

    await page.locator('cb-sidebar').getByRole('link', { name: /All Contacts/ }).click();
    await expect(page).toHaveURL('/contacts');
    await expectSidebarGroupsStable(page);

    await page.locator('cb-sidebar').getByRole('link', { name: /Favorites/ }).click();
    await expect(page).toHaveURL('/favorites');
    await expectSidebarGroupsStable(page);
  });

  test('sidebar active state updates across navigation', async ({ page }) => {
    await page.goto('/');
    await expectActiveSidebarNav(page, 'Dashboard');

    await page.locator('cb-sidebar').getByRole('link', { name: /Favorites/ }).click();
    await expect(page).toHaveURL('/favorites');
    await expect(page.locator('cb-page-favorites .page-title')).toHaveText('Favorites');
    await expectActiveSidebarNav(page, 'Favorites');

    await page.locator('cb-sidebar').getByRole('link', { name: 'Work' }).click();
    await expect(page).toHaveURL('/groups/Work');
    await expect(page.locator('cb-page-group .page-title')).toContainText('Work');
    await expectActiveSidebarNav(page, 'Work');

    await page.locator('cb-sidebar').getByRole('link', { name: /All Contacts/ }).click();
    await expect(page).toHaveURL('/contacts');
    await expect(page.locator('cb-page-contacts .page-title')).toHaveText('All Contacts');
    await expectActiveSidebarNav(page, 'All Contacts');
  });

  test('navigate to group via sidebar', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('link', { name: 'Work' }).first().click();
    await expect(page).toHaveURL('/groups/Work');
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

  test('contact detail favorite action toggles state and sidebar count', async ({ page }) => {
    await page.goto('/contacts/1');

    const favoriteButton = page.locator('cb-contact-detail .favorite-btn');
    const favoritesCount = page.locator('cb-sidebar [data-nav="Favorites"] .nav-count');
    const initialPressed = await favoriteButton.getAttribute('aria-pressed');
    const initialCount = Number.parseInt((await favoritesCount.textContent()) ?? '', 10);

    expect(initialPressed === 'true' || initialPressed === 'false').toBe(true);
    expect(Number.isNaN(initialCount)).toBe(false);

    const toggledPressed = initialPressed === 'true' ? 'false' : 'true';
    const toggledCount = String(initialCount + (initialPressed === 'true' ? -1 : 1));
    const restoredCount = String(initialCount);

    await expect(page).toHaveURL('/contacts/1');
    await expect(favoriteButton).toHaveAttribute('aria-pressed', initialPressed!);
    await expect(favoritesCount).toHaveText(restoredCount);
    await expectSidebarGroupsStable(page);
    await expectContactDetailFieldsStable(page);

    await favoriteButton.click();
    await expect(page).toHaveURL('/contacts/1');
    await expect(favoriteButton).toHaveAttribute('aria-pressed', toggledPressed);
    await expect(favoritesCount).toHaveText(toggledCount);
    await expectSidebarGroupsStable(page);
    await expectContactDetailFieldsStable(page);

    await favoriteButton.click();
    await expect(favoriteButton).toHaveAttribute('aria-pressed', initialPressed!);
    await expect(favoritesCount).toHaveText(restoredCount);
    await expectSidebarGroupsStable(page);
    await expectContactDetailFieldsStable(page);
  });

  test('does not reload the document for scriptless route navigation', async ({ page }) => {
    await page.goto('/');
    await page.evaluate(() => { (window as any).__testMarker = Date.now(); });

    await page.locator('cb-sidebar').getByRole('link', { name: /All Contacts/ }).click();
    await expect(page.locator('cb-page-contacts .page-title')).toHaveText('All Contacts');

    await page.locator('cb-sidebar').getByRole('link', { name: /Favorites/ }).click();
    await expect(page.locator('cb-page-favorites .page-title')).toHaveText('Favorites');
    expect(await page.evaluate(() => (window as any).__testMarker)).toBeGreaterThan(0);
  });
});

// ── Regression: sidebar groups stable after contact edit (issue #177) ─

test.describe('contact edit does not corrupt sidebar groups', () => {
  test('editing a contact group from Work to Friends keeps sidebar labels stable', async ({ page, request }) => {
    // Ensure contact #1 starts in group "Work" (reset from any prior test run)
    await request.put('http://127.0.0.1:3013/api/contacts/1', { data: { group: 'Work' } });

    // Navigate to contact #1 (Sarah Chen, group: Work)
    await page.goto('/contacts/1');
    await expect(page.locator('cb-contact-detail')).toBeVisible();
    await expect(page.locator('cb-contact-detail .badge')).toHaveText('Work');

    // Verify sidebar groups before edit
    await expectSidebarGroupsStable(page);

    // Click edit
    await page.locator('cb-contact-detail .icon-edit').click();
    await expect(page).toHaveURL('/contacts/1/edit');
    await expect(page.locator('cb-contact-form .form-title')).toHaveText('Edit Contact');

    // Change group from Work to Friends
    const friendsLabel = page.locator('cb-contact-form .group-radio-label', { hasText: 'Friends' });
    await friendsLabel.click();

    // Save the contact
    await page.locator('cb-contact-form .save-btn').click();
    await expect(page).toHaveURL('/contacts/1');

    // Sidebar groups must remain in the same stable order
    await expectSidebarGroupsStable(page);

    // The contact detail now shows Friends group
    await expect(page.locator('cb-contact-detail .badge')).toHaveText('Friends');

    // Navigate to Work group — should still exist and be clickable
    await page.locator('cb-sidebar').getByRole('link', { name: 'Work' }).click();
    await expect(page).toHaveURL('/groups/Work');
    await expect(page.locator('cb-page-group .page-title')).toContainText('Work');

    // Restore contact back to Work for test isolation
    await page.goto('/contacts/1/edit');
    await page.locator('cb-contact-form .group-radio-label', { hasText: 'Work' }).click();
    await page.locator('cb-contact-form .save-btn').click();
    await expect(page).toHaveURL('/contacts/1');
    await expectSidebarGroupsStable(page);
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
