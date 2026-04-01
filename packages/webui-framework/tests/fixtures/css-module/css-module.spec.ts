// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

for (const mode of ['light', 'shadow'] as const) {
test.describe(`css module fixture [${mode} DOM]`, () => {
  test.beforeEach(async ({ page }) => {
    const file = mode === 'light' ? 'fixture.html' : 'fixture-shadow.html';
    await page.goto(`/css-module/${file}`);
    await page.waitForSelector('test-module-host .spawn');
  });

  test('client-created components adopt module styles from registered specifiers', async ({ page }) => {
    const before = await page.locator('test-module-host').evaluate((host) => {
      const label = (host.shadowRoot ?? host).querySelector('.host-label');
      // In shadow DOM mode, stylesheets are on shadowRoot.
      // In light DOM mode, styles are injected into document head.
      const sr = host.shadowRoot;
      const sheetCount = sr
        ? sr.adoptedStyleSheets?.length ?? 0
        : document.querySelectorAll('style').length > 0 ? 1 : 0;
      return {
        hasStyles: sheetCount > 0,
        hostColor: label instanceof HTMLElement ? getComputedStyle(label).color : null,
      };
    });

    expect(before.hasStyles).toBe(true);
    expect(before.hostColor).toBe('rgb(0, 102, 204)');

    await page.locator('test-module-host .spawn').click();

    const after = await page.locator('test-module-host').evaluate((host) => {
      const child = (host.shadowRoot ?? host).querySelector('test-module-child');
      const label = (child?.shadowRoot ?? child)?.querySelector('.child-label');
      return {
        childColor: label instanceof HTMLElement ? getComputedStyle(label).color : null,
      };
    });

    expect(after.childColor).toBe('rgb(178, 34, 34)');
  });
});
}
