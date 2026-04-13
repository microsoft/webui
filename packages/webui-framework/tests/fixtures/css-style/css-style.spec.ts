// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('css style fixture', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/css-style/fixture.html');
    await page.waitForSelector('test-style-host .spawn');
  });

  test('client-created components preserve inline style tags in compiled static html', async ({ page }) => {
    const before = await page.locator('test-style-host').evaluate((host) => ({
      hostStyleCount: (host.shadowRoot ?? host).querySelectorAll('style').length ?? 0,
      hostColor: (() => {
        const label = (host.shadowRoot ?? host).querySelector('.host-label');
        return label instanceof HTMLElement ? getComputedStyle(label).color : null;
      })(),
    }));

    expect(before).toEqual({
      hostStyleCount: 1,
      hostColor: 'rgb(12, 34, 56)',
    });

    await page.locator('test-style-host .spawn').click();

    const after = await page.locator('test-style-host').evaluate((host) => {
      const child = (host.shadowRoot ?? host).querySelector('test-style-child');
      const label = (child?.shadowRoot ?? child)?.querySelector('.child-label');
      return {
        childStyleCount: (child?.shadowRoot ?? child)?.querySelectorAll('style').length ?? 0,
        childColor: label instanceof HTMLElement ? getComputedStyle(label).color : null,
      };
    });

    expect(after).toEqual({
      childStyleCount: 1,
      childColor: 'rgb(210, 105, 30)',
    });
  });
});
