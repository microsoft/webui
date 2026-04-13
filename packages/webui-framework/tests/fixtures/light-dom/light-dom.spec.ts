// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Tests light-DOM hydration — the framework code path where components
 * render without shadow DOM.  The pipeline always produces shadow DOM,
 * so this fixture uses manual template registration and hand-written
 * SSR HTML to keep the light-DOM path covered.
 */

import { expect, test } from '@playwright/test';

test.describe('light-dom hydration', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/light-dom/fixture.html');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-light-dom');
      return el && (el as any).$ready === true;
    });
  });

  test('hydrates SSR text content in light DOM', async ({ page }) => {
    await expect(page.locator('test-light-dom .greeting')).toHaveText('Hello');
    await expect(page.locator('test-light-dom .name')).toHaveText('World');
  });

  test('does NOT create a shadow root', async ({ page }) => {
    const hasShadow = await page.evaluate(() =>
      !!document.querySelector('test-light-dom')?.shadowRoot,
    );
    expect(hasShadow).toBe(false);
  });

  test('updates @observable reactively in light DOM', async ({ page }) => {
    await page.evaluate(() => {
      (document.querySelector('test-light-dom') as any).greeting = 'Hi';
      (document.querySelector('test-light-dom') as any).name = 'WebUI';
    });

    await expect(page.locator('test-light-dom .greeting')).toHaveText('Hi');
    await expect(page.locator('test-light-dom .name')).toHaveText('WebUI');
  });
});
