// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('text fixture', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/text/fixture.html');
    await page.waitForSelector('test-text');
  });

  test('renders SSR text content', async ({ page }) => {
    await expect(page.locator('test-text .greeting')).toHaveText('Hello');
    await expect(page.locator('test-text .name')).toHaveText('World');
  });

  test('removes SSR markers after hydration', async ({ page }) => {
    const result = await page.evaluate(() => {
      const host = document.querySelector('test-text');
      const root = host?.shadowRoot;
      if (!root) {
        return { comments: ['missing-shadow-root'], attrs: ['missing-shadow-root'] };
      }

      const comments = [];
      const walker = document.createTreeWalker(root, NodeFilter.SHOW_COMMENT);
      let current;
      while ((current = walker.nextNode())) {
        comments.push((current as Comment).data);
      }

      const attrs = [];
      for (const el of Array.from(root.querySelectorAll('*'))) {
        for (const attr of Array.from(el.attributes) as Attr[]) {
          if (attr.name.startsWith('data-w-')) {
            attrs.push(attr.name);
          }
        }
      }

      return { comments, attrs };
    });

    expect(result.comments).toEqual([]);
    expect(result.attrs).toEqual([]);
  });

  test('updates @attr text reactively', async ({ page }) => {
    await page.evaluate(() => {
      document.querySelector('test-text')?.setAttribute('greeting', 'Hi');
    });

    await expect(page.locator('test-text .greeting')).toHaveText('Hi');
  });

  test('updates @observable text reactively', async ({ page }) => {
    await page.evaluate(() => {
      const host = document.querySelector('test-text') as { name: string } | null;
      if (host) {
        host.name = 'Playwright';
      }
    });

    await expect(page.locator('test-text .name')).toHaveText('Playwright');
  });

  test('fires the hydration completion event', async ({ page }) => {
    const duration = await page.evaluate(() => new Promise<number>((resolve) => {
      const existing = performance.getEntriesByName('webui:hydrate:total', 'measure');
      if (existing.length > 0) {
        resolve(existing[0].duration);
        return;
      }

      window.addEventListener('webui:hydration-complete', () => {
        const measures = performance.getEntriesByName('webui:hydrate:total', 'measure');
        resolve(measures[0]?.duration ?? -1);
      }, { once: true });
    }));

    expect(duration).toBeGreaterThanOrEqual(0);
  });
});
