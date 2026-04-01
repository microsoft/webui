// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

for (const mode of ['light', 'shadow'] as const) {
test.describe(`optional css link fixture [${mode} DOM]`, () => {
  test.beforeEach(async ({ page }) => {
    const file = mode === 'light' ? 'fixture.html' : 'fixture-shadow.html';
    await page.goto(`/optional-css-link/${file}`);
    await page.waitForSelector('test-no-css-host .spawn');
  });

  test('client-created components skip stylesheet links when no CSS was discovered', async ({ page }) => {
    const before = await page.locator('test-no-css-host').evaluate((host) => ({
      hostHref: (host.shadowRoot ?? host).querySelector('link[rel="stylesheet"]')?.getAttribute('href') ?? null,
    }));

    expect(before.hostHref).toBeNull();

    await page.locator('test-no-css-host .spawn').click();

    const after = await page.locator('test-no-css-host').evaluate((host) => {
      const child = (host.shadowRoot ?? host).querySelector('test-no-css-child');
      return {
        hostHref: (host.shadowRoot ?? host).querySelector('link[rel="stylesheet"]')?.getAttribute('href') ?? null,
        childExists: !!child,
        childHref: (child?.shadowRoot ?? child)?.querySelector('link[rel="stylesheet"]')?.getAttribute('href') ?? null,
        childText: (child?.shadowRoot ?? child)?.querySelector('.child-label')?.textContent ?? null,
      };
    });

    expect(after).toEqual({
      hostHref: null,
      childExists: true,
      childHref: null,
      childText: 'Ready',
    });
  });
});
}
