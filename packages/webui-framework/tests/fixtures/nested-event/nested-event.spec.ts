// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

for (const mode of ['light', 'shadow'] as const) {
test.describe(`nested event fixture [${mode} DOM]`, () => {
  test.beforeEach(async ({ page }) => {
    const file = mode === 'light' ? 'fixture.html' : 'fixture-shadow.html';
    await page.goto(`/nested-event/${file}`);
    await page.waitForSelector('test-nested-event');
  });

  test('parent hydrates events correctly with nested child component', async ({ page }) => {
    const errors: string[] = [];
    page.on('pageerror', (err) => errors.push(err.message));

    const result = await page.evaluate(() => {
      const host = document.querySelector('test-nested-event') as any;
      const root = host?.shadowRoot ?? host;
      const btn = root?.querySelector('.parent-btn');
      btn?.dispatchEvent(new MouseEvent('click', { bubbles: true, composed: true }));
      host?.$flushUpdates();
      return host?.parentClicks ?? -1;
    });

    expect(result).toBe(1);
    expect(errors).toEqual([]);
  });
});
}
