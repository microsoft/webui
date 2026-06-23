// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test, type Page } from '@playwright/test';

type LazyResource = 'asset' | 'css' | 'data' | 'module';

interface WebUIWindow {
  __webui?: {
    templates?: Record<string, unknown>;
  };
}

function classifyLazyResource(url: string): LazyResource | undefined {
  const { pathname } = new URL(url);
  if (pathname.endsWith('/lazy-panel.webui.js')) return 'asset';
  if (pathname.endsWith('/lazy-panel.css')) return 'css';
  if (pathname.endsWith('/lazy-panel-data.json')) return 'data';
  if (pathname.includes('/chunks/lazy-panel-') && pathname.endsWith('.js')) {
    return 'module';
  }
  return undefined;
}

function countLazyRequests(requests: LazyResource[], resource: LazyResource): number {
  return requests.filter((item) => item === resource).length;
}

async function loadedTemplateNames(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const webui = (window as typeof window & WebUIWindow).__webui;
    return Object.keys(webui?.templates ?? {}).sort();
  });
}

test.describe('static component assets', () => {
  test('loads lazy assets only after interaction and reuses cached templates', async ({ page }) => {
    const lazyRequests: LazyResource[] = [];
    page.on('request', (request) => {
      const resource = classifyLazyResource(request.url());
      if (resource) lazyRequests.push(resource);
    });

    await page.goto('/');
    await expect(page.getByRole('button', { name: 'Load lazy panel' })).toBeVisible();
    await expect(page.locator('lazy-panel')).toHaveCount(0);

    expect(lazyRequests).toEqual([]);
    expect(await loadedTemplateNames(page)).not.toContain('lazy-panel');

    await page.getByRole('button', { name: 'Load lazy panel' }).click();
    await expect(page.locator('lazy-panel')).toHaveCount(1);
    await expect(page.getByText('Static asset template is active')).toBeVisible();
    await expect(page.getByText('Loaded from component fetch')).toBeVisible();

    expect(await loadedTemplateNames(page)).toContain('lazy-panel');
    expect(countLazyRequests(lazyRequests, 'asset')).toBe(1);
    expect(countLazyRequests(lazyRequests, 'module')).toBe(1);
    expect(countLazyRequests(lazyRequests, 'data')).toBe(1);
    expect(countLazyRequests(lazyRequests, 'css')).toBeGreaterThanOrEqual(1);

    const firstLoadCounts = {
      asset: countLazyRequests(lazyRequests, 'asset'),
      module: countLazyRequests(lazyRequests, 'module'),
      data: countLazyRequests(lazyRequests, 'data'),
    };

    await page.getByRole('button', { name: 'Load lazy panel' }).click();
    await expect(page.locator('lazy-panel')).toHaveCount(1);
    await expect(page.getByText('Static asset template is active')).toBeVisible();

    expect(countLazyRequests(lazyRequests, 'asset')).toBe(firstLoadCounts.asset);
    expect(countLazyRequests(lazyRequests, 'module')).toBe(firstLoadCounts.module);
    expect(countLazyRequests(lazyRequests, 'data')).toBe(firstLoadCounts.data);
  });
});
