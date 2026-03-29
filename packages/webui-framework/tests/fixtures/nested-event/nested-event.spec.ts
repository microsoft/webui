// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('nested event fixture', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/nested-event/fixture.html');
    await page.waitForSelector('test-nested-event');
  });

  test('parent hydrates without errors when child has data-ev markers', async ({ page }) => {
    const errors: string[] = [];
    page.on('pageerror', (err) => errors.push(err.message));

    const result = await page.evaluate(() => {
      const host = document.querySelector('test-nested-event') as any;
      const btn = host?.shadowRoot?.querySelector('.parent-btn');
      btn?.dispatchEvent(new MouseEvent('click', { bubbles: true, composed: true }));
      return host?.parentClicks ?? -1;
    });

    expect(result).toBe(1);
    expect(errors).toEqual([]);
  });
});
