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
    await expect(page.locator('nav a')).toHaveCount(10);
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

test.describe('ensureLoaded — non-route components', () => {
  test('ensureLoaded registers a component template from the server', async ({ page }) => {
    await page.goto('/');
    await page.waitForFunction(() => {
      const el = document.querySelector('route-shell');
      return el && (el as any).$ready === true;
    });

    // test-dialog is NOT in the route tree and NOT eagerly imported
    const beforeLoad = await page.evaluate(() => {
      return !!window.__webui_templates?.['test-dialog'];
    });
    // Template may or may not be pre-registered from SSR build discovery,
    // but the component class should NOT be defined yet
    const definedBefore = await page.evaluate(() => {
      return !!customElements.get('test-dialog');
    });
    expect(definedBefore).toBe(false);

    // Call ensureLoaded — should fetch template from /_webui/templates
    const result = await page.evaluate(async () => {
      const router = (window as any).__testRouter;
      await router.ensureLoaded('test-dialog');
      return !!window.__webui_templates?.['test-dialog'];
    });
    expect(result).toBe(true);
  });

  test('ensureLoaded is idempotent — second call is instant', async ({ page }) => {
    await page.goto('/');
    await page.waitForFunction(() => {
      const el = document.querySelector('route-shell');
      return el && (el as any).$ready === true;
    });

    // First call
    await page.evaluate(async () => {
      const router = (window as any).__testRouter;
      await router.ensureLoaded('test-dialog');
    });

    // Second call should return instantly (no network)
    const start = await page.evaluate(async () => {
      const t0 = performance.now();
      const router = (window as any).__testRouter;
      await router.ensureLoaded('test-dialog');
      return performance.now() - t0;
    });

    // Should complete quickly — no network round-trip (< 50ms on CI)
    expect(start).toBeLessThan(50);
  });

  test('ensureLoaded supports batch loading multiple components', async ({ page }) => {
    await page.goto('/');
    await page.waitForFunction(() => {
      const el = document.querySelector('route-shell');
      return el && (el as any).$ready === true;
    });

    // Batch-load (test-dialog is the only non-route component, but the
    // call should handle it alongside already-loaded route components)
    const result = await page.evaluate(async () => {
      const router = (window as any).__testRouter;
      await router.ensureLoaded('test-dialog', 'page-alpha');
      return {
        dialog: !!window.__webui_templates?.['test-dialog'],
        alpha: !!window.__webui_templates?.['page-alpha'],
      };
    });
    expect(result.dialog).toBe(true);
    expect(result.alpha).toBe(true);
  });
});

test.describe('keep-alive state preservation', () => {
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

  test('keep-alive preserves local state across navigations', async ({ page }) => {
    // Navigate to the keep-alive page
    await page.click('a[href="/keepalive"]');
    await expect(page.locator('.counter')).toContainText('Counter: 0');

    // Increment the counter (local state change)
    await page.click('.increment');
    await expect(page.locator('.counter')).toContainText('Counter: 1');
    await page.click('.increment');
    await expect(page.locator('.counter')).toContainText('Counter: 2');

    // Navigate away to alpha
    await page.click('a[href="/alpha"]');
    await expect(page.locator('page-alpha h2')).toContainText('Alpha Page');

    // Navigate back to keep-alive page
    await page.click('a[href="/keepalive"]');
    await expect(page.locator('.counter')).toContainText('Counter: 2',
      { timeout: 3000 },
    );
  });
});

test.describe('route loaders', () => {
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

  test('static loader provides component state on SPA navigation', async ({ page }) => {
    // Navigate to loader page
    await page.click('a[href="/loader"]');
    await expect(page.locator('.source')).toContainText('Source: client-loader');
    await expect(page.locator('.loader-data')).toContainText('Data fetched by static loader');
  });

  test('X-WebUI-Has-Loader header is sent when leaf has loader', async ({ page }) => {
    // First navigate to the loader page so the router discovers its loader
    await page.click('a[href="/loader"]');
    await expect(page.locator('.source')).toContainText('Source: client-loader');

    // Now navigate again — the router should send the X-WebUI-Has-Loader header
    const [request] = await Promise.all([
      page.waitForRequest(req =>
        req.url().includes('/alpha') &&
        req.headers()['accept']?.includes('application/json'),
      ),
      page.click('a[href="/alpha"]'),
    ]);

    // The header should be present since page-loader was the previous leaf
    // and it has a static loader. Note: the header signals the PREVIOUS
    // leaf had a loader, which tells the server the current nav might too.
    // The actual value depends on whether the router detected loaders.
    const hasLoaderHeader = request.headers()['x-webui-has-loader'];
    expect(hasLoaderHeader).toBeDefined();
    expect(hasLoaderHeader).toContain('page-loader');
  });
});

test.describe('pending UI', () => {
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

  test('shows pending skeleton during slow navigation then replaces with real content', async ({ page }) => {
    // Intercept the partial JSON fetch for /slow and delay it by 500ms
    await page.route('**/slow', async (route) => {
      const request = route.request();
      if (request.headers()['accept']?.includes('application/json')) {
        // Delay the response to trigger pending UI (threshold is 150ms)
        await new Promise(r => setTimeout(r, 500));
        await route.continue();
      } else {
        await route.continue();
      }
    });

    // Navigate to the slow page
    await page.click('a[href="/slow"]');

    // The loading skeleton element should appear after ~150ms
    await expect(page.locator('loading-skeleton')).toBeVisible({ timeout: 3000 });

    // After the fetch completes (~500ms), real content should replace the skeleton
    await expect(page.locator('[data-testid="page-slow"]')).toBeVisible({ timeout: 5000 });
    await expect(page.locator('h2')).toContainText('Slow Page');
  });

  test('skips pending for fast navigations', async ({ page }) => {
    // No fetch delay — navigation should be fast enough to skip pending
    let skeletonSeen = false;

    // Monitor for the skeleton element
    page.on('console', (msg) => {
      if (msg.text().includes('skeleton-mounted')) skeletonSeen = true;
    });

    await page.evaluate(() => {
      const observer = new MutationObserver((mutations) => {
        for (const m of mutations) {
          for (const node of m.addedNodes) {
            if ((node as Element)?.tagName === 'LOADING-SKELETON') {
              console.log('skeleton-mounted');
            }
          }
        }
      });
      observer.observe(document.body, { childList: true, subtree: true });
    });

    await page.click('a[href="/slow"]');
    await expect(page.locator('[data-testid="page-slow"]')).toBeVisible({ timeout: 5000 });

    // The skeleton should NOT have appeared for a fast navigation
    expect(skeletonSeen).toBe(false);
  });
});

test.describe('error boundaries', () => {
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

  test('shows error component when navigation fetch fails', async ({ page }) => {
    // Intercept the partial JSON fetch for /failing and return a 500 error
    await page.route('**/failing', async (route) => {
      const request = route.request();
      if (request.headers()['accept']?.includes('application/json')) {
        await route.fulfill({
          status: 500,
          contentType: 'text/plain',
          body: 'Internal Server Error',
        });
      } else {
        await route.continue();
      }
    });

    // Navigate to the failing page
    await page.click('a[href="/failing"]');

    // The error display element should be mounted
    await expect(page.locator('error-display')).toBeVisible({ timeout: 5000 });
  });

  test('error component does not appear for successful navigations', async ({ page }) => {
    // Normal navigation to alpha — no error should appear
    await page.click('a[href="/alpha"]');
    await expect(page.locator('h2')).toContainText('Alpha Page');

    // Error display should not be in the DOM
    const errorCount = await page.locator('[data-testid="error-display"]').count();
    expect(errorCount).toBe(0);
  });
});
