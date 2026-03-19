// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * End-to-end tests for the WebUI nested routing example.
 *
 * Tests both SSR (direct page loads) and client-side navigation
 * (clicking links). Validates that:
 * - Correct content renders at each nesting level
 * - Parent content persists during child navigation
 * - URL updates correctly
 * - Route tree JSON is embedded for client use
 */

import { test, expect } from '@playwright/test';

// ── SSR Tests (direct page loads) ────────────────────────────────

test.describe('SSR routing', () => {
  test('root page renders shell with section links', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('h1')).toHaveText('Learning Platform');
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
    expect(data.chain[1].path).toBe('sections/:sectionId');
  });

  test('section page renders section content via SSR', async ({ page }) => {
    await page.goto('/sections/frontend');
    // Shell persists
    await expect(page.locator('h1')).toHaveText('Learning Platform');
    // Section content (inside main, not nav)
    await expect(page.locator('main h2')).toContainText('Frontend');
    // Topic links from Express API
    await expect(page.getByRole('link', { name: 'React' })).toBeVisible();
    await expect(page.getByRole('link', { name: 'CSS' })).toBeVisible();
  });

  test('topic page renders at 3 levels via SSR', async ({ page }) => {
    await page.goto('/sections/backend/topics/rust');
    await expect(page.locator('h1')).toHaveText('Learning Platform');
    await expect(page.locator('main h2')).toContainText('Backend');
    await expect(page.locator('main h3')).toContainText('Rust');
    await expect(page.getByRole('link', { name: 'Ownership' })).toBeVisible();
    await expect(page.getByRole('link', { name: 'Traits' })).toBeVisible();
  });

  test('lesson page renders at 4 levels via SSR', async ({ page }) => {
    await page.goto('/sections/frontend/topics/react/lessons/hooks');
    await expect(page.locator('h1')).toHaveText('Learning Platform');
    await expect(page.locator('main h2')).toContainText('Frontend');
    await expect(page.locator('main h3')).toContainText('React');
    await expect(page.locator('main h4')).toContainText('React Hooks');
    await expect(page.locator('body')).toContainText(
      'Hooks let you use state and lifecycle features in function components.'
    );
  });

  test('webui-route elements have correct active state in SSR', async ({ page }) => {
    const html = await (await page.goto('/sections/frontend'))!.text();
    // Root route active
    expect(html).toContain('path="/" component="routes-app" active>');
    // Section route active
    expect(html).toContain('path="sections/:sectionId" component="section-page" active>');
    // Topic route hidden (not in URL)
    expect(html).toContain('component="topic-page" style="display:none">');
  });

  test('child routes are inside main element', async ({ page }) => {
    const html = await (await page.goto('/sections/frontend'))!.text();
    const mainIdx = html.indexOf('<main>');
    const routeIdx = html.indexOf('<webui-route path="sections');
    expect(routeIdx).toBeGreaterThan(mainIdx);
  });
});

// ── Client-side navigation tests ─────────────────────────────────

test.describe('client-side navigation', () => {
  test('navigate root → section', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('link', { name: 'Frontend' }).click();
    await expect(page).toHaveURL('/sections/frontend');
    // Shell preserved
    await expect(page.locator('h1')).toHaveText('Learning Platform');
    // Section loaded
    await expect(page.locator('main h2')).toContainText('Frontend');
    await expect(page.getByRole('link', { name: 'React' })).toBeVisible();
  });

  test('navigate section → topic', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('link', { name: 'Frontend' }).click();
    await expect(page.locator('main h2')).toContainText('Frontend');

    await page.getByRole('link', { name: 'React' }).click();
    await expect(page).toHaveURL('/sections/frontend/topics/react');
    // Shell + section preserved
    await expect(page.locator('h1')).toHaveText('Learning Platform');
    await expect(page.locator('main h2')).toContainText('Frontend');
    // Topic loaded
    await expect(page.locator('main h3')).toContainText('React');
    await expect(page.getByRole('link', { name: 'Introduction to React' })).toBeVisible();
  });

  test('navigate topic → lesson (4 levels deep)', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('link', { name: 'Frontend' }).click();
    await expect(page.locator('main h2')).toContainText('Frontend');

    await page.getByRole('link', { name: 'React' }).click();
    await expect(page.locator('main h3')).toContainText('React');

    await page.getByRole('link', { name: 'React Hooks' }).click();
    await expect(page).toHaveURL('/sections/frontend/topics/react/lessons/hooks');
    // All parent content preserved
    await expect(page.locator('h1')).toHaveText('Learning Platform');
    await expect(page.locator('main h2')).toContainText('Frontend');
    await expect(page.locator('main h3')).toContainText('React');
    // Lesson content
    await expect(page.locator('main h4')).toContainText('React Hooks');
    await expect(page.locator('body')).toContainText('function components');
  });

  test('navigate between siblings preserves parent', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('link', { name: 'Frontend' }).click();
    await expect(page.locator('main h2')).toContainText('Frontend');

    await page.getByRole('link', { name: 'React' }).click();
    await expect(page.locator('main h3')).toContainText('React');

    // Navigate to CSS (sibling topic)
    await page.getByRole('link', { name: 'CSS' }).click();
    await expect(page).toHaveURL('/sections/frontend/topics/css');
    // Shell + section preserved
    await expect(page.locator('h1')).toHaveText('Learning Platform');
    await expect(page.locator('main h2')).toContainText('Frontend');
    // Topic changed
    await expect(page.locator('main h3')).toContainText('CSS');
    await expect(page.getByRole('link', { name: 'Flexbox Layout' })).toBeVisible();
  });

  test('navigate to different section', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('link', { name: 'Frontend' }).click();
    await expect(page.locator('main h2')).toContainText('Frontend');

    // Click Backend in nav
    await page.getByRole('link', { name: 'Backend' }).click();
    await expect(page).toHaveURL('/sections/backend');
    await expect(page.locator('main h2')).toContainText('Backend');
    await expect(page.getByRole('link', { name: 'Rust' })).toBeVisible();
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

// ── Screenshot tests (via client-side navigation) ────────────────

test.describe('visual regression (client navigation)', () => {
  test('root page screenshot', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('h1')).toHaveText('Learning Platform');
    await expect(page).toHaveScreenshot('root-page.png', { maxDiffPixelRatio: 0.01 });
  });

  test('section page via click screenshot', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('link', { name: 'Frontend' }).click();
    await expect(page.locator('main h2')).toContainText('Frontend');
    await expect(page).toHaveScreenshot('section-frontend-click.png', { maxDiffPixelRatio: 0.01 });
  });

  test('topic page via click screenshot', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('link', { name: 'Frontend' }).click();
    await expect(page.locator('main h2')).toContainText('Frontend');
    await page.getByRole('link', { name: 'React' }).click();
    await expect(page.locator('main h3')).toContainText('React');
    await expect(page).toHaveScreenshot('topic-react-click.png', { maxDiffPixelRatio: 0.01 });
  });

  test('lesson page via click screenshot', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('link', { name: 'Frontend' }).click();
    await expect(page.locator('main h2')).toContainText('Frontend');
    await page.getByRole('link', { name: 'React' }).click();
    await expect(page.locator('main h3')).toContainText('React');
    await page.getByRole('link', { name: 'React Hooks' }).click();
    await expect(page.locator('main h4')).toContainText('React Hooks');
    await expect(page).toHaveScreenshot('lesson-hooks-click.png', { maxDiffPixelRatio: 0.01 });
  });
});
