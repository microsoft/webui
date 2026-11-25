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

  test('declarative-only lesson page auto-upgrades without an authored stub', async ({ page }) => {
    await page.goto('/sections/frontend/topics/react/lessons/hooks');
    const lesson = page.locator('lesson-page');
    await expect(lesson).toBeVisible();

    await expect(
      lesson.evaluate((el) => {
        const component = el as HTMLElement & { $ready?: boolean; setState?: unknown };

        return {
          ready: component.$ready === true,
          setState: typeof component.setState === 'function',
        };
      }),
    ).resolves.toEqual({ ready: true, setState: true });

    await lesson.evaluate((el) => {
      const component = el as HTMLElement & { setState(state: unknown): void };
      component.setState({
        sectionName: 'Frontend',
        topicName: 'React',
        lessonName: 'Fallback lesson',
        lessonContent: 'Updated through router fallback state.',
      });
    });
    await expect(lesson).toContainText('Fallback lesson');
    await expect(lesson).toContainText('Updated through router fallback state.');
  });

  test('webui-route elements have correct active state in SSR', async ({ page }) => {
    const html = await (await page.goto('/sections/frontend'))!.text();
    expect(html).toContain('path="/" component="routes-app" data-ri="0" active>');
    expect(html).toContain('path="sections/:sectionId" component="section-page" data-ri="1" active>');
    expect(html).toContain('component="topic-page" style="display:none">');
  });

  test('only rendered component templates emitted on initial page', async ({ page }) => {
    // Root page should only have routes-app template, not all 4
    await page.goto('/');
    const rootTemplates = await page.evaluate(
      () => Object.keys(window.__webui?.templates ?? {}),
    );
    expect(rootTemplates).toEqual(['routes-app']);

    // Section page should have routes-app + section-page
    await page.goto('/sections/frontend');
    const sectionTemplates = await page.evaluate(
      () => Object.keys(window.__webui?.templates ?? {}).sort(),
    );
    expect(sectionTemplates).toEqual(['routes-app', 'section-page']);

    // Deep page should have all 4
    await page.goto('/sections/frontend/topics/react/lessons/hooks');
    const deepTemplates = await page.evaluate(
      () => Object.keys(window.__webui?.templates ?? {}).sort(),
    );
    expect(deepTemplates).toEqual(['lesson-page', 'routes-app', 'section-page', 'topic-page']);
  });

  test('partial response delivers missing templates during client navigation', async ({ page }) => {
    await page.goto('/');
    // Only routes-app on root
    let templates = await page.evaluate(() => Object.keys(window.__webui?.templates ?? {}));
    expect(templates).toEqual(['routes-app']);

    // Navigate to Frontend — section-page template should arrive via partial
    await page.getByRole('link', { name: 'Frontend' }).click();
    await expect(page.locator('main h2')).toContainText('Frontend');
    templates = await page.evaluate(() => Object.keys(window.__webui?.templates ?? {}).sort());
    expect(templates).toContain('section-page');

    // Navigate to React — topic-page template should arrive
    await page.getByRole('link', { name: 'React' }).click();
    await expect(page.locator('main h3')).toContainText('React');
    templates = await page.evaluate(() => Object.keys(window.__webui?.templates ?? {}).sort());
    expect(templates).toContain('topic-page');
  });
});

// ── Client-side navigation tests ─────────────────────────────────

test.describe('client-side navigation', () => {
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

});

// ── Interactive island tests (counters) ──────────────────────────

test.describe('interactive islands', () => {
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
  test('active attribute updates when navigating to different section', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('link', { name: 'Frontend' }).click();
    await expect(page.locator('main h2')).toContainText('Frontend');

    await page.getByRole('link', { name: 'Backend' }).click();
    await expect(page.locator('main h2')).toContainText('Backend');

    // Backend link should be active, Frontend should not.
    const nav = page.locator('routes-app > nav');
    await expect(nav.getByRole('link', { name: /Backend/ })).toHaveAttribute('active', '');
    await expect(nav.getByRole('link', { name: /Frontend/ })).not.toHaveAttribute('active', '');
  });

  test('topic link active attribute updates on topic navigation', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('link', { name: 'Frontend' }).click();
    await page.getByRole('link', { name: 'React' }).click();
    await expect(page.locator('main h3')).toContainText('React');

    // The unwrapped Light DOM topic link for React should be active.
    const react = page.locator('section-page .topics').getByRole('link', { name: 'React' });
    await expect(react).toHaveAttribute('active', '');
  });
});
