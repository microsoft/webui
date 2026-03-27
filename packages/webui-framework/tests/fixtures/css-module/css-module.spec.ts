// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('css module fixture', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/css-module/fixture.html');
    await page.waitForSelector('test-module-host .spawn');
  });

  test('client-created components adopt module styles from registered specifiers', async ({ page }) => {
    const before = await page.locator('test-module-host').evaluate((host) => {
      const label = host.shadowRoot?.querySelector('.host-label');
      return {
        hostStylesheetCount: host.shadowRoot?.adoptedStyleSheets.length ?? 0,
        hostColor: label instanceof HTMLElement ? getComputedStyle(label).color : null,
      };
    });

    expect(before).toEqual({
      hostStylesheetCount: 1,
      hostColor: 'rgb(0, 102, 204)',
    });

    await page.locator('test-module-host .spawn').click();

    const after = await page.locator('test-module-host').evaluate((host) => {
      const child = host.shadowRoot?.querySelector('test-module-child');
      const label = child?.shadowRoot?.querySelector('.child-label');
      return {
        childStylesheetCount: child?.shadowRoot?.adoptedStyleSheets.length ?? 0,
        childColor: label instanceof HTMLElement ? getComputedStyle(label).color : null,
      };
    });

    expect(after).toEqual({
      childStylesheetCount: 1,
      childColor: 'rgb(178, 34, 34)',
    });
  });
});
