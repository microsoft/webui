// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('HTML-only dormant host fixture', () => {
  let warnings: string[];

  test.beforeEach(async ({ page }) => {
    warnings = [];
    page.on('console', (message) => {
      if (message.type() === 'warning') warnings.push(message.text());
    });
    await page.goto('/html-only-ssr/fixture.html');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-html-only');
      return el && (el as unknown as { $ready?: boolean }).$ready === true;
    });
  });

  test('registers without consuming bootstrap state or hydrating SSR DOM', async ({ page }) => {
    await expect(page.locator('script[src="/dist/static-host.js"]')).toHaveCount(1);
    expect(await page.evaluate(() => customElements.get('test-html-only') !== undefined)).toBe(true);

    await expect(page.locator('test-html-only .heading')).toHaveText('Contacts');
    await expect(page.locator('test-html-only .filter')).toHaveText('all');
    await expect(page.locator('test-html-only .status')).toHaveText('Ready');
    await expect(page.locator('test-html-only .detail-link')).toHaveAttribute('href', '/items/42');
    await expect(page.locator('test-html-only .item')).toHaveText(['Ada', 'Grace']);
    await expect(page.locator('test-html-only .details')).toHaveCount(0);
    const data = await page.evaluate(() => ({
      state: window.__webui?.state,
      templates: window.__webui?.templates,
    }));
    expect(data.state).toEqual({});
    expect(Object.keys(data.templates ?? {})).toEqual(['test-html-only']);
    expect(await page.evaluate(
      () => performance.getEntriesByName('webui:hydrate:total', 'measure').length,
    )).toBe(0);
  });

  test('activates on the first browser state write', async ({ page }) => {
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
    expect(await page.evaluate(
      () => performance.getEntriesByName('webui:hydrate:total', 'measure').length,
    )).toBe(1);
  });

  test('preserves untouched SSR bindings when the first write omits their roots', async ({ page }) => {
    await page.evaluate(() => {
      const host = document.querySelector('test-html-only') as {
        setState(state: Record<string, unknown>): void;
      } | null;
      host?.setState({ status: 'Partially updated' });
    });

    await expect(page.locator('test-html-only .status')).toHaveText('Partially updated');
    await expect(page.locator('test-html-only .heading')).toHaveText('Contacts');
    await expect(page.locator('test-html-only .filter')).toHaveText('all');
    await expect(page.locator('test-html-only .detail-link')).toHaveAttribute('href', '/items/42');
    await expect(page.locator('test-html-only .item')).toHaveText(['Ada', 'Grace']);
    await expect(page.locator('test-html-only .details')).toHaveCount(0);
    expect(warnings.filter((warning) => warning.includes('repeat marker count'))).toHaveLength(0);
  });

  test('removes excess SSR repeat items on first activation', async ({ page }) => {
    await page.evaluate(() => {
      const host = document.querySelector('test-html-only') as {
        setState(state: Record<string, unknown>): void;
      } | null;
      host?.setState({
        heading: 'One contact',
        status: 'Loaded',
        selectedId: '7',
        items: [{ name: 'Radia' }],
        showDetails: false,
        details: '',
      });
    });

    await expect(page.locator('test-html-only .item')).toHaveText(['Radia']);
    expect(warnings.filter((warning) => warning.includes('repeat marker count'))).toHaveLength(0);
  });
});
