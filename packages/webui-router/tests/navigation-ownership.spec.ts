// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';
import type { NavigationCache, PartialResponse } from '../src/cache.js';
import type { WebUIRouter } from '../src/router.js';

type TestRouter = Pick<WebUIRouter, 'navigate' | 'activeComponent'> & {
  navCache: NavigationCache | null;
  startupNavigation: Promise<void> | null;
  partialControllers: Set<AbortController>;
  deferredReader: Promise<void> | null;
};

declare global {
  interface Window {
    __testRouter: TestRouter;
  }
}

test('zero freshness releases completed JSON responses while every revisit refetches', async ({
  page,
  browserName,
}) => {
  const requests = new Map<string, number>();
  await page.route('**/items/*', async route => {
    if (!route.request().headers().accept?.includes('application/json')) {
      await route.continue();
      return;
    }
    const path = new URL(route.request().url()).pathname;
    const revision = (requests.get(path) ?? 0) + 1;
    requests.set(path, revision);
    const response = await route.fetch();
    const data = await response.json() as PartialResponse;
    const item = path.slice('/items/'.length);
    await route.fulfill({
      response,
      headers: { ...response.headers(), 'cache-control': 'no-store' },
      json: {
        ...data,
        state: {
          ...data.state,
          itemId: `${item} revision ${revision}`,
          records: Array.from({ length: 64 }, (_, n) => ({
            id: `${item}-${revision}-${n}`,
            title: `Customer project ${item}/${n}`,
            permissions: ['read', 'comment'],
            description: `${item}/${revision}/${n}:` + 'Representative project details. '.repeat(60),
          })),
        },
      },
    });
  });
  await page.clock.setFixedTime(new Date('2026-11-03T03:00:00Z'));
  await page.goto('/beta?cache=zero');
  await page.waitForFunction(() => window.__testRouter?.activeComponent === 'page-beta'
    && window.__testRouter.startupNavigation === null);

  for (let item = 1; item <= 32; item++) {
    await page.evaluate(async item => {
      const navigated = new Promise(resolve => {
        window.addEventListener('webui:route:navigated', resolve, { once: true });
      });
      window.__testRouter.navigate(`/items/${item}`);
      await navigated;
    }, item);
    await expect(page.locator('h2')).toHaveText(`Item ${item} revision 1`);
  }
  await page.evaluate(async () => {
    await Promise.all(document.getAnimations().map(animation => animation.finished));
  });
  await page.waitForFunction(() => window.__testRouter.partialControllers.size === 0
    && window.__testRouter.deferredReader === null);
  const retained = await page.evaluate(() => {
    const cache = window.__testRouter.navCache;
    let responses = 0;
    for (let item = 1; item <= 32; item++) {
      if (cache?.has(`/items/${item}`)) responses++;
    }
    return responses;
  });
  expect(requests.size).toBe(32);
  expect([...requests.values()]).toEqual(new Array(32).fill(1));

  if (browserName === 'chromium') {
    const cdp = await page.context().newCDPSession(page);
    const heap = [];
    for (let n = 0; n < 3; n++) {
      await cdp.send('HeapProfiler.collectGarbage');
      heap.push(await cdp.send('Runtime.getHeapUsage'));
    }
    await test.info().attach('completed-navigation-live-heap', {
      body: JSON.stringify({ navigations: 32, retained, heap }),
      contentType: 'application/json',
    });
  }
  expect(retained).toBe(0);

  // The clock is still the exact store timestamp: zero means no ordinary reuse.
  await page.evaluate(async () => {
    const navigated = new Promise(resolve => {
      window.addEventListener('webui:route:navigated', resolve, { once: true });
    });
    window.__testRouter.navigate('/items/1');
    await navigated;
  });
  await expect(page.locator('h2')).toHaveText('Item 1 revision 2');
  expect(requests.get('/items/1')).toBe(2);
});

test('zero freshness preserves native link keyboard navigation', async ({ page, browserName }) => {
  await page.goto('/beta?cache=zero');
  await page.waitForFunction(() => window.__testRouter?.activeComponent === 'page-beta'
    && window.__testRouter.startupNavigation === null);

  // Safari on macOS uses Option-Tab to include links in keyboard traversal.
  const tab = browserName === 'webkit' && process.platform === 'darwin' ? 'Alt+Tab' : 'Tab';
  const previous = browserName === 'webkit' && process.platform === 'darwin' ? 'Alt+Shift+Tab' : 'Shift+Tab';
  await page.keyboard.press(tab);
  await expect(page.locator('nav a[href="/"]')).toBeFocused();
  await page.keyboard.press(tab);
  await expect(page.locator('nav a[href="/projects"]')).toBeFocused();
  await page.keyboard.press(tab);
  await expect(page.locator('nav a[href="/alpha"]')).toBeFocused();
  await page.keyboard.press(previous);
  await expect(page.locator('nav a[href="/projects"]')).toBeFocused();
  await page.keyboard.press(tab);
  await page.keyboard.press('Enter');
  await expect(page.locator('h2')).toHaveText('Alpha Page');
  expect(await page.evaluate(() => window.__testRouter.activeComponent)).toBe('page-alpha');
});
