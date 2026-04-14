// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Router E2E tests — browser-level routing behaviors.
 *
 * These tests exercise the WebUI Router in a real browser against a
 * multi-route fixture app served by the WebUI CLI.  They focus on
 * behaviors that unit tests cannot cover:
 *
 * - Browser back/forward history navigation
 * - Deep-link SSR + reload preservation
 * - Client-side navigation without full page reload
 * - Route parameter rendering
 */

import { test, expect } from '@playwright/test';

test.describe('SSR deep links', () => {
  test('root page renders shell with nav links', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('h1')).toContainText('Router Test');
    await expect(page.locator('nav a')).toHaveCount(6);
  });

  test('alpha page renders via SSR', async ({ page }) => {
    await page.goto('/alpha');
    await expect(page.locator('h2')).toContainText('Alpha Page');
    await expect(page.locator('.content')).toContainText('Welcome to the Alpha page');
  });

  test('beta page renders via SSR', async ({ page }) => {
    await page.goto('/beta');
    await expect(page.locator('h2')).toContainText('Beta Page');
  });

  test('parameterized route renders item detail via SSR', async ({ page }) => {
    await page.goto('/items/42');
    await expect(page.locator('h2')).toContainText('Item 42');
    await expect(page.locator('.content')).toContainText('Detail for item 42');
  });
});

test.describe('client-side navigation', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/');
    // Wait for hydration + router start
    await page.waitForFunction(() => {
      const el = document.querySelector('route-shell');
      return el && (el as any).$ready === true;
    }, null);
    // Wait for Router.start() to register the navigate listener
    await page.waitForFunction(
      () => !!(window as any).navigation,
      null,
      { timeout: 5000 },
    );
    await page.waitForTimeout(300);
  });

  test('navigates without full page reload', async ({ page }) => {
    // Mark the shell element to detect full reloads
    await page.evaluate(() => {
      (document.querySelector('route-shell') as any).__marker = true;
    });

    await page.locator('nav a[href="/alpha"]').click();
    await expect(page.locator('main h2')).toContainText('Alpha Page');

    // Shell marker should survive — no full reload
    const survived = await page.evaluate(() =>
      (document.querySelector('route-shell') as any).__marker === true,
    );
    expect(survived).toBe(true);
  });

  // TODO: sibling navigation unmounts old page but fails to mount new one.
  // The router fetches the correct partial (with template) but doesn't
  // render the new component after the first client navigation.
  test.fixme('navigates between sibling routes', async ({ page }) => {
    await page.locator('nav a[href="/alpha"]').click();
    await expect(page.locator('main h2')).toContainText('Alpha Page');

    await page.locator('nav a[href="/beta"]').click();
    await expect(page.locator('main h2')).toContainText('Beta Page');
  });

  test('navigates to parameterized route', async ({ page }) => {
    await page.locator('nav a[href="/items/1"]').click();
    await expect(page.locator('main h2')).toContainText('Item 1');
  });
});

test.describe('browser history', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/');
    await page.waitForFunction(() => {
      const el = document.querySelector('route-shell');
      return el && (el as any).$ready === true;
    }, null);
    await page.waitForFunction(
      () => !!(window as any).navigation,
      null,
      { timeout: 5000 },
    );
    await page.waitForTimeout(300);
  });

  // TODO: depends on sibling navigation fix (see fixme above)
  test.fixme('back button returns to previous route', async ({ page }) => {
    await page.locator('nav a[href="/alpha"]').click();
    await expect(page.locator('main h2')).toContainText('Alpha Page');

    await page.goBack();
    await expect(page.locator('page-alpha')).toHaveCount(0);
  });

  // TODO: depends on sibling navigation fix (see fixme above)
  test.fixme('forward button restores navigated route', async ({ page }) => {
    await page.locator('nav a[href="/beta"]').click();
    await expect(page.locator('main h2')).toContainText('Beta Page');

    await page.goBack();
    await expect(page.locator('page-beta')).toHaveCount(0);

    await page.goForward();
    await expect(page.locator('main h2')).toContainText('Beta Page');
  });

  // TODO: depends on sibling navigation fix (see fixme above)
  test.fixme('multi-step history traversal works correctly', async ({ page }) => {
    await page.locator('nav a[href="/alpha"]').click();
    await expect(page.locator('main h2')).toContainText('Alpha Page');

    await page.locator('nav a[href="/beta"]').click();
    await expect(page.locator('main h2')).toContainText('Beta Page');

    await page.locator('nav a[href="/items/1"]').click();
    await expect(page.locator('main h2')).toContainText('Item 1');

    await page.goBack();
    await expect(page.locator('main h2')).toContainText('Beta Page');

    await page.goBack();
    await expect(page.locator('main h2')).toContainText('Alpha Page');
  });
});

test.describe('page reload preservation', () => {
  test('reloading a deep route re-renders via SSR', async ({ page }) => {
    await page.goto('/alpha');
    await expect(page.locator('h2')).toContainText('Alpha Page');

    await page.reload();
    await expect(page.locator('h2')).toContainText('Alpha Page');
    await expect(page.locator('h1')).toContainText('Router Test');
  });

  test('reloading a parameterized route preserves the parameter', async ({ page }) => {
    await page.goto('/items/99');
    await expect(page.locator('h2')).toContainText('Item 99');

    await page.reload();
    await expect(page.locator('h2')).toContainText('Item 99');
  });
});

test.describe('query parameter passing', () => {
  test('SSR renders compose page (query params not available server-side)', async ({ page }) => {
    await page.goto('/compose?action=reply&to=test@example.com&subject=Re: Hello');
    await expect(page.locator('h2')).toContainText('Compose');
  });

  test('client-side navigation passes query params as attributes', async ({ page }) => {
    await page.goto('/');
    await page.waitForFunction(() => {
      const el = document.querySelector('route-shell');
      return el && (el as any).$ready === true;
    }, null);
    await page.waitForFunction(
      () => !!(window as any).navigation,
      null,
      { timeout: 5000 },
    );
    await page.waitForTimeout(300);

    // Click the compose link with query params
    await page.locator('nav a[href*="compose"]').click();
    await expect(page.locator('h2')).toContainText('Compose');

    // Verify query params were set as attributes on the component
    const action = await page.locator('.action').textContent();
    expect(action).toContain('reply');

    const to = await page.locator('.to').textContent();
    expect(to).toContain('test@example.com');

    const subject = await page.locator('.subject').textContent();
    expect(subject).toContain('Re: Hello');
  });

  test('navigated event includes query object', async ({ page }) => {
    await page.goto('/');
    await page.waitForFunction(() => {
      const el = document.querySelector('route-shell');
      return el && (el as any).$ready === true;
    }, null);
    await page.waitForFunction(
      () => !!(window as any).navigation,
      null,
      { timeout: 5000 },
    );
    await page.waitForTimeout(300);

    // Listen for the navigated event
    const queryPromise = page.evaluate(() => {
      return new Promise<Record<string, string>>((resolve) => {
        window.addEventListener('webui:route:navigated', ((e: CustomEvent) => {
          resolve(e.detail.query);
        }) as EventListener, { once: true });
      });
    });

    await page.locator('nav a[href*="compose"]').click();
    const query = await queryPromise;
    expect(query).toEqual({
      action: 'reply',
      to: 'test@example.com',
      subject: 'Re: Hello',
    });
  });

  test('unlisted query params are NOT set as attributes (allowlist enforcement)', async ({ page }) => {
    await page.goto('/');
    await page.waitForFunction(() => {
      const el = document.querySelector('route-shell');
      return el && (el as any).$ready === true;
    }, null);
    await page.waitForFunction(
      () => !!(window as any).navigation,
      null,
      { timeout: 5000 },
    );
    await page.waitForTimeout(300);

    // Navigate to compose with both allowed and disallowed query params
    await page.evaluate(() => {
      (window as any).navigation.navigate('/compose?action=reply&to=user@test.com&class=evil&style=display:none&id=hijack');
    });
    await expect(page.locator('h2')).toContainText('Compose');

    // Allowed params should be set
    const comp = page.locator('page-compose');
    await expect(comp).toHaveAttribute('action', 'reply');
    await expect(comp).toHaveAttribute('to', 'user@test.com');

    // Disallowed params must NOT be set
    expect(await comp.getAttribute('class')).toBeNull();
    expect(await comp.getAttribute('style')).toBeNull();
    expect(await comp.getAttribute('id')).toBeNull();
  });

  test('navigated event includes ALL query params (unfiltered)', async ({ page }) => {
    await page.goto('/');
    await page.waitForFunction(() => {
      const el = document.querySelector('route-shell');
      return el && (el as any).$ready === true;
    }, null);
    await page.waitForFunction(
      () => !!(window as any).navigation,
      null,
      { timeout: 5000 },
    );
    await page.waitForTimeout(300);

    const queryPromise = page.evaluate(() => {
      return new Promise<Record<string, string>>((resolve) => {
        window.addEventListener('webui:route:navigated', ((e: CustomEvent) => {
          resolve(e.detail.query);
        }) as EventListener, { once: true });
      });
    });

    await page.evaluate(() => {
      (window as any).navigation.navigate('/compose?action=reply&evil=inject');
    });

    const query = await queryPromise;
    // Event should contain ALL params (unfiltered) for JS consumers
    expect(query).toEqual({ action: 'reply', evil: 'inject' });
  });
});
