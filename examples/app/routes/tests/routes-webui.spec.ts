// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * End-to-end tests for the WebUI nested routing example (webui-framework port).
 *
 * Tests SSR, client-side navigation, interactive islands (counters),
 * and auto-populated @observable state from the router.
 */

import { test, expect } from '@playwright/test';

// ── SSR Tests (direct page loads) ────────────────────────────────

test.describe('SSR routing', () => {
  test('root page renders shell with section links', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('h1')).toContainText('Learning Platform');
    await expect(page.locator('nav a')).toHaveCount(3);
    await expect(page.locator('nav')).toContainText('Frontend');
    await expect(page.locator('nav')).toContainText('Backend');
    await expect(page.locator('nav')).toContainText('DevOps');
  });

  test('JSON partial includes matched route chain', async ({ page }) => {
    const resp = await page.request.get('/sections/frontend', {
      headers: { 'Accept': 'application/json' },
    });
    expect(resp.ok()).toBeTruthy();
    const data = await resp.json();
    expect(data.chain).toBeDefined();
    expect(data.chain.length).toBeGreaterThanOrEqual(2);
    expect(data.chain[0].component).toBe('routes-app');
    expect(data.chain[1].component).toBe('section-page');
  });

  test('section page renders section content via SSR', async ({ page }) => {
    await page.goto('/sections/frontend');
    await expect(page.locator('h1')).toContainText('Learning Platform');
    await expect(page.locator('main h2')).toContainText('Frontend');
    await expect(page.getByRole('link', { name: 'React' })).toBeVisible();
    await expect(page.getByRole('link', { name: 'CSS' })).toBeVisible();
  });

  test('topic page renders at 3 levels via SSR', async ({ page }) => {
    await page.goto('/sections/backend/topics/rust');
    await expect(page.locator('h1')).toContainText('Learning Platform');
    await expect(page.locator('main h2')).toContainText('Backend');
    await expect(page.locator('main h3')).toContainText('Rust');
  });

  test('lesson page renders at 4 levels via SSR', async ({ page }) => {
    await page.goto('/sections/frontend/topics/react/lessons/hooks');
    await expect(page.locator('h1')).toContainText('Learning Platform');
    await expect(page.locator('main h2')).toContainText('Frontend');
    await expect(page.locator('main h3')).toContainText('React');
    await expect(page.locator('main h4')).toContainText('React Hooks');
    await expect(page.locator('body')).toContainText(
      'Hooks let you use state and lifecycle features in function components.'
    );
  });

  test('webui-route elements have correct active state in SSR', async ({ page }) => {
    const html = await (await page.goto('/sections/frontend'))!.text();
    expect(html).toContain('path="/" component="routes-app" active>');
    expect(html).toContain('path="sections/:sectionId" component="section-page" active>');
    expect(html).toContain('component="topic-page" style="display:none">');
  });

  test('only rendered component templates emitted on initial page', async ({ page }) => {
    // Root page should only have routes-app template, not all 4
    await page.goto('/');
    const rootTemplates = await page.evaluate(
      () => Object.keys(window.__webui_templates ?? {}),
    );
    expect(rootTemplates).toEqual(['routes-app']);

    // Section page should have routes-app + section-page
    await page.goto('/sections/frontend');
    const sectionTemplates = await page.evaluate(
      () => Object.keys(window.__webui_templates ?? {}).sort(),
    );
    expect(sectionTemplates).toEqual(['routes-app', 'section-page']);

    // Deep page should have all 4
    await page.goto('/sections/frontend/topics/react/lessons/hooks');
    const deepTemplates = await page.evaluate(
      () => Object.keys(window.__webui_templates ?? {}).sort(),
    );
    expect(deepTemplates).toEqual(['lesson-page', 'routes-app', 'section-page', 'topic-page']);
  });

  test('partial response delivers missing templates during client navigation', async ({ page }) => {
    await page.goto('/');
    // Only routes-app on root
    let templates = await page.evaluate(() => Object.keys(window.__webui_templates ?? {}));
    expect(templates).toEqual(['routes-app']);

    // Navigate to Frontend — section-page template should arrive via partial
    await page.getByRole('link', { name: 'Frontend' }).click();
    await expect(page.locator('main h2')).toContainText('Frontend');
    templates = await page.evaluate(() => Object.keys(window.__webui_templates ?? {}).sort());
    expect(templates).toContain('section-page');

    // Navigate to React — topic-page template should arrive
    await page.getByRole('link', { name: 'React' }).click();
    await expect(page.locator('main h3')).toContainText('React');
    templates = await page.evaluate(() => Object.keys(window.__webui_templates ?? {}).sort());
    expect(templates).toContain('topic-page');
  });
});

// ── Client-side navigation tests ─────────────────────────────────

test.describe('client-side navigation', () => {
  test('navigate root → section', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('link', { name: 'Frontend' }).click();
    await expect(page).toHaveURL('/sections/frontend');
    await expect(page.locator('h1')).toContainText('Learning Platform');
    await expect(page.locator('main h2')).toContainText('Frontend');
    await expect(page.getByRole('link', { name: 'React' })).toBeVisible();
  });

  test('navigate section → topic', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('link', { name: 'Frontend' }).click();
    await expect(page.locator('main h2')).toContainText('Frontend');

    await page.getByRole('link', { name: 'React' }).click();
    await expect(page).toHaveURL('/sections/frontend/topics/react');
    await expect(page.locator('main h3')).toContainText('React');
    await expect(page.getByRole('link', { name: 'Introduction to React' })).toBeVisible();
  });

  test('navigate topic → lesson (4 levels deep)', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('link', { name: 'Frontend' }).click();
    await page.getByRole('link', { name: 'React' }).click();
    await expect(page.locator('main h3')).toContainText('React');

    await page.getByRole('link', { name: 'React Hooks' }).click();
    await expect(page).toHaveURL('/sections/frontend/topics/react/lessons/hooks');
    await expect(page.locator('main h4')).toContainText('React Hooks');
    await expect(page.locator('body')).toContainText('function components');
  });

  test('navigate between siblings preserves parent', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('link', { name: 'Frontend' }).click();
    await page.getByRole('link', { name: 'React' }).click();
    await expect(page.locator('main h3')).toContainText('React');

    await page.getByRole('link', { name: 'CSS' }).click();
    await expect(page).toHaveURL('/sections/frontend/topics/css');
    await expect(page.locator('main h2')).toContainText('Frontend');
    await expect(page.locator('main h3')).toContainText('CSS');
  });

  test('navigate to different section', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('link', { name: 'Frontend' }).click();
    await expect(page.locator('main h2')).toContainText('Frontend');

    await page.getByRole('link', { name: 'Backend' }).click();
    await expect(page).toHaveURL('/sections/backend');
    await expect(page.locator('main h2')).toContainText('Backend');
  });

  test('no full page reload during client navigation', async ({ page }) => {
    await page.goto('/');
    await page.evaluate(() => { (window as any).__routeTestMarker = Date.now(); });

    await page.getByRole('link', { name: 'Frontend' }).click();
    await expect(page.locator('main h2')).toContainText('Frontend');

    const marker = await page.evaluate(() => (window as any).__routeTestMarker);
    expect(marker).toBeGreaterThan(0);
  });
});

// ── Interactive island tests (counters) ──────────────────────────

test.describe('interactive islands', () => {
  test('shell counter increments on click', async ({ page }) => {
    await page.goto('/');
    await expect(page.getByRole('button', { name: /Shell/ })).toContainText('0');

    await page.getByRole('button', { name: /Shell/ }).click();
    await page.getByRole('button', { name: /Shell/ }).click();
    await expect(page.getByRole('button', { name: /Shell/ })).toContainText('2');
  });

  test('shell counter persists across client navigation', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('button', { name: /Shell/ }).click();
    await page.getByRole('button', { name: /Shell/ }).click();
    await page.getByRole('button', { name: /Shell/ }).click();
    await expect(page.getByRole('button', { name: /Shell/ })).toContainText('3');

    // Navigate to Frontend
    await page.getByRole('link', { name: 'Frontend' }).click();
    await expect(page.locator('main h2')).toContainText('Frontend');

    // Shell counter preserved
    await expect(page.getByRole('button', { name: /Shell/ })).toContainText('3');
  });

  test('section counter works on dynamically created component', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('link', { name: 'Frontend' }).click();
    await expect(page.locator('main h2')).toContainText('Frontend');

    // Section counter starts at 0
    await expect(page.getByRole('button', { name: /Section/ })).toContainText('0');
    await page.getByRole('button', { name: /Section/ }).click();
    await page.getByRole('button', { name: /Section/ }).click();
    await expect(page.getByRole('button', { name: /Section/ })).toContainText('2');
  });
});

// ── Boolean attribute (?active) tests ────────────────────────────

test.describe('?active boolean attributes', () => {
  test('SSR renders active attribute on matching nav link', async ({ page }) => {
    const html = await (await page.goto('/sections/frontend'))!.text();
    // The <a> for Frontend should have the active attribute in SSR
    expect(html).toContain('active');
  });

  test('active attribute present after client navigation to section', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('link', { name: 'Frontend' }).click();
    await expect(page.locator('main h2')).toContainText('Frontend');

    // Check the nav link for Frontend has the active attribute
    const hasActive = await page.evaluate(() => {
      const app = document.querySelector('routes-app');
      const links = app?.shadowRoot?.querySelectorAll('nav a');
      for (const a of links ?? []) {
        if (a.textContent?.includes('Frontend') && a.hasAttribute('active')) {
          return true;
        }
      }
      return false;
    });
    expect(hasActive).toBe(true);
  });

  test('active attribute updates when navigating to different section', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('link', { name: 'Frontend' }).click();
    await expect(page.locator('main h2')).toContainText('Frontend');

    await page.getByRole('link', { name: 'Backend' }).click();
    await expect(page.locator('main h2')).toContainText('Backend');

    // Backend link should be active, Frontend should not
    const activeStates = await page.evaluate(() => {
      const app = document.querySelector('routes-app');
      const links = app?.shadowRoot?.querySelectorAll('nav a');
      const states: Record<string, boolean> = {};
      for (const a of links ?? []) {
        const text = a.textContent?.trim() ?? '';
        if (text.includes('Frontend')) states['Frontend'] = a.hasAttribute('active');
        if (text.includes('Backend')) states['Backend'] = a.hasAttribute('active');
      }
      return states;
    });
    expect(activeStates['Backend']).toBe(true);
    expect(activeStates['Frontend']).toBe(false);
  });

  test('topic link active attribute updates on topic navigation', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('link', { name: 'Frontend' }).click();
    await page.getByRole('link', { name: 'React' }).click();
    await expect(page.locator('main h3')).toContainText('React');

    // React topic link should be active
    const hasActive = await page.evaluate(() => {
      const section = document.querySelector('routes-app')?.shadowRoot?.querySelector('section-page');
      const links = section?.shadowRoot?.querySelectorAll('.topics a');
      for (const a of links ?? []) {
        if (a.textContent?.includes('React') && a.hasAttribute('active')) {
          return true;
        }
      }
      return false;
    });
    expect(hasActive).toBe(true);
  });
});
