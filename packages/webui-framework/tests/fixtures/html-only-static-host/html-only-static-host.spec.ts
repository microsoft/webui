// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('html-only static host fixture', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/html-only-static-host/fixture.html');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-html-only');
      return el && (el as unknown as { $ready?: boolean }).$ready === true;
    });
  });

  test('hydrates an HTML-only component without a component stub', async ({ page }) => {
    await expect(page.locator('script[src="/dist/html-only-static-host/element.js"]')).toHaveCount(0);

    const hasFallback = await page.evaluate(() => {
      const el = document.querySelector('test-html-only') as {
        setState?: (state: Record<string, unknown>) => void;
      } | null;
      return customElements.get('test-html-only') !== undefined &&
        typeof el?.setState === 'function';
    });
    expect(hasFallback).toBe(true);

    await expect(page.locator('test-html-only .heading')).toHaveText('Contacts');
    await expect(page.locator('test-html-only .filter')).toHaveText('all');
    await expect(page.locator('test-html-only .status')).toHaveText('Ready');
    await expect(page.locator('test-html-only .detail-link')).toHaveAttribute('href', '/items/42');
    await expect(page.locator('test-html-only .item')).toHaveText(['Ada', 'Grace']);
    await expect(page.locator('test-html-only .details')).toHaveCount(0);
  });

  test('updates template bindings through setState', async ({ page }) => {
    await page.evaluate(() => {
      const host = document.querySelector('test-html-only') as {
        setState(state: Record<string, unknown>): void;
      } | null;
      host?.setState({
        heading: 'Updated contacts',
        status: 'Loaded',
        selectedId: '99',
        items: [
          { name: 'Linus' },
          { name: 'Margaret' },
          { name: 'Radia' },
        ],
        showDetails: true,
        details: 'Loaded from state',
      });
    });

    await expect(page.locator('test-html-only .heading')).toHaveText('Updated contacts');
    await expect(page.locator('test-html-only .status')).toHaveText('Loaded');
    await expect(page.locator('test-html-only .detail-link')).toHaveAttribute('href', '/items/99');
    await expect(page.locator('test-html-only .item')).toHaveText(['Linus', 'Margaret', 'Radia']);
    await expect(page.locator('test-html-only .details')).toHaveText('Loaded from state');
  });

  test('updates template bindings when host attributes change', async ({ page }) => {
    await page.evaluate(() => {
      const host = document.querySelector('test-html-only');
      host?.setAttribute('filter', 'favorites');
      host?.setAttribute('heading', 'Attribute heading');
    });

    await expect(page.locator('test-html-only .filter')).toHaveText('favorites');
    await expect(page.locator('test-html-only .heading')).toHaveText('Attribute heading');
  });
});
