// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test, type Page } from '@playwright/test';

type LazyResource =
  | 'lazy-asset'
  | 'secondary-asset'
  | 'shared-chunk'
  | 'css'
  | 'data'
  | 'module';

interface WebUIWindow {
  __webui?: {
    templates?: Record<string, unknown>;
  };
}

function classifyLazyResource(url: string): LazyResource | undefined {
  const { pathname } = new URL(url);
  if (pathname.endsWith('/lazy-panel.webui.js')) return 'lazy-asset';
  if (pathname.endsWith('/secondary-panel.webui.js')) return 'secondary-asset';
  if (pathname.endsWith('/chunk-shared-detail.webui.js')) return 'shared-chunk';
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
  test('starts generated Link styles before the root asset settles', async ({ page }) => {
    let cssRequests = 0;
    let releaseAsset!: () => void;
    let assetRequested!: () => void;
    let cssRequested!: () => void;
    const assetGate = new Promise<void>(resolve => {
      releaseAsset = resolve;
    });
    const assetSeen = new Promise<void>(resolve => {
      assetRequested = resolve;
    });
    const cssSeen = new Promise<void>(resolve => {
      cssRequested = resolve;
    });
    await page.route('**/lazy-panel.webui.js', async route => {
      assetRequested();
      await assetGate;
      await route.continue();
    });
    await page.route('**/lazy-panel.css', async route => {
      cssRequests += 1;
      cssRequested();
      await route.continue();
    });

    await page.goto('/');
    const button = page.getByRole('button', { name: 'Load lazy panel' });
    await expect(button).toBeVisible();
    await button.hover();
    await Promise.all([assetSeen, cssSeen]);
    await expect(page.locator('lazy-panel')).toHaveCount(0);

    await button.click();
    releaseAsset();
    await expect(page.locator('lazy-panel')).toHaveCount(1);
    expect(cssRequests).toBe(1);
  });

  test('waits for Link stylesheet cache warmup before inserting a lazy component', async ({ page }) => {
    let releaseCss!: () => void;
    let cssRequested!: () => void;
    const cssGate = new Promise<void>(resolve => {
      releaseCss = resolve;
    });
    const requestSeen = new Promise<void>(resolve => {
      cssRequested = resolve;
    });
    await page.route('**/lazy-panel.css', async route => {
      cssRequested();
      await cssGate;
      await route.continue();
    });

    await page.goto('/');
    await expect(page.getByRole('button', { name: 'Load lazy panel' })).toBeVisible();
    await page.getByRole('button', { name: 'Load lazy panel' }).click();
    await requestSeen;
    await page.evaluate(async () => {
      await new Promise<void>(resolve =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve()))
      );
    });
    await expect(page.locator('lazy-panel')).toHaveCount(0);

    releaseCss();
    await expect(page.locator('lazy-panel')).toHaveCount(1);
    await expect(page.locator('lazy-panel').evaluate((panel) => ({
      adopted: panel.shadowRoot?.adoptedStyleSheets.length ?? 0,
      disabled:
        (panel.shadowRoot?.querySelector(
          'link[rel~="stylesheet"]',
        ) as HTMLLinkElement | null)?.disabled ?? false,
      links: panel.shadowRoot?.querySelectorAll('link[rel~="stylesheet"]').length ?? 0,
    }))).resolves.toEqual({ adopted: 1, disabled: true, links: 1 });
  });

  test('splits and reuses a lazy-only dependency chunk', async ({ page }) => {
    const lazyRequests: LazyResource[] = [];
    page.on('request', (request) => {
      const resource = classifyLazyResource(request.url());
      if (resource) lazyRequests.push(resource);
    });

    await page.goto('/');
    await expect(page.getByRole('button', { name: 'Load lazy panel' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Load secondary panel' })).toBeVisible();
    await expect(page.locator('lazy-panel')).toHaveCount(0);
    await expect(page.locator('secondary-panel')).toHaveCount(0);
    const badge = page.locator('asset-badge').first();
    await expect(
      badge.evaluate((el) => {
        const component = el as HTMLElement & { $ready?: boolean; setState?: unknown };

        return {
          ready: component.$ready === true,
          setState: typeof component.setState === 'function',
        };
      }),
    ).resolves.toEqual({ ready: true, setState: true });

    expect(lazyRequests).toEqual([]);
    expect(await loadedTemplateNames(page)).not.toContain('lazy-panel');
    expect(await loadedTemplateNames(page)).not.toContain('secondary-panel');
    expect(await loadedTemplateNames(page)).not.toContain('shared-detail');

    await page.getByRole('button', { name: 'Load secondary panel' }).click();
    await expect(page.locator('secondary-panel')).toHaveCount(1);
    await expect(page.getByText('Secondary component asset')).toBeVisible();
    await expect(page.getByText('Shared lazy dependency is active')).toBeVisible();

    expect(countLazyRequests(lazyRequests, 'secondary-asset')).toBe(1);
    expect(countLazyRequests(lazyRequests, 'shared-chunk')).toBe(1);
    expect(countLazyRequests(lazyRequests, 'lazy-asset')).toBe(0);

    await page.getByRole('button', { name: 'Load lazy panel' }).click();
    await expect(page.locator('lazy-panel')).toHaveCount(1);
    const lazyPanel = page.locator('lazy-panel').first();
    await expect(
      lazyPanel.evaluate((el) => {
        const component = el as HTMLElement & { $ready?: boolean; setState?: unknown };

        return {
          ready: component.$ready === true,
          setState: typeof component.setState === 'function',
        };
      }),
    ).resolves.toEqual({ ready: true, setState: true });
    await expect(page.getByText('Static asset template is active')).toBeVisible();
    await expect(page.getByText('Loaded from component fetch')).toBeVisible();

    await lazyPanel.evaluate((el) => {
      const component = el as HTMLElement & { setState(state: unknown): void };
      component.setState({
        status: 'Updated',
        heading: 'Fallback lazy panel',
        message: 'Updated through setState()',
        hasDetails: true,
        details: 'No authored lazy panel class required.',
      });
    });
    await expect(lazyPanel).toContainText('Fallback lazy panel');
    await expect(lazyPanel).toContainText('No authored lazy panel class required.');

    expect(await loadedTemplateNames(page)).toContain('lazy-panel');
    expect(await loadedTemplateNames(page)).toContain('secondary-panel');
    expect(await loadedTemplateNames(page)).toContain('shared-detail');
    expect(countLazyRequests(lazyRequests, 'lazy-asset')).toBe(1);
    expect(countLazyRequests(lazyRequests, 'secondary-asset')).toBe(1);
    expect(countLazyRequests(lazyRequests, 'shared-chunk')).toBe(1);
    expect(countLazyRequests(lazyRequests, 'module')).toBe(0);
    expect(countLazyRequests(lazyRequests, 'data')).toBe(1);
    expect(countLazyRequests(lazyRequests, 'css')).toBeGreaterThanOrEqual(1);

    const firstLoadCounts = {
      asset: countLazyRequests(lazyRequests, 'lazy-asset'),
      data: countLazyRequests(lazyRequests, 'data'),
    };

    await page.getByRole('button', { name: 'Load lazy panel' }).click();
    await expect(page.locator('lazy-panel')).toHaveCount(1);
    await expect(page.getByText('Static asset template is active')).toBeVisible();

    expect(countLazyRequests(lazyRequests, 'lazy-asset')).toBe(firstLoadCounts.asset);
    expect(countLazyRequests(lazyRequests, 'shared-chunk')).toBe(1);
    expect(countLazyRequests(lazyRequests, 'module')).toBe(0);
    expect(countLazyRequests(lazyRequests, 'data')).toBe(firstLoadCounts.data);
  });
});
