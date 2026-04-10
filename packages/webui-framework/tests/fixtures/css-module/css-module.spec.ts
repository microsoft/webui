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
    // Wait for async CSS module injection (import().then() in injectModuleStyle)
    await expect(async () => {
      const hostColor = await page.locator('test-module-host').evaluate((host) => {
        const label = (host.shadowRoot ?? host).querySelector('.host-label');
        return label instanceof HTMLElement ? getComputedStyle(label).color : null;
      });
      expect(hostColor).toBe('rgb(0, 102, 204)');
    }).toPass({ timeout: 5_000 });

    await page.locator('test-module-host .spawn').click();

    // Wait for async CSS module adoption on the dynamically-created child
    await expect(async () => {
      const childColor = await page.locator('test-module-host').evaluate((host) => {
        const child = (host.shadowRoot ?? host).querySelector('test-module-child');
        const label = (child?.shadowRoot ?? child)?.querySelector('.child-label');
        return label instanceof HTMLElement ? getComputedStyle(label).color : null;
      });
      expect(childColor).toBe('rgb(178, 34, 34)');
    }).toPass({ timeout: 5_000 });
  });
});
}
