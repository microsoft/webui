// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

for (const mode of ['light', 'shadow'] as const) {
test.describe(`ref fixture [${mode} DOM]`, () => {
  test.beforeEach(async ({ page }) => {
    const file = mode === 'light' ? 'fixture.html' : 'fixture-shadow.html';
    await page.goto(`/ref/${file}`);
    await page.waitForFunction(
      () => (document.querySelector('test-ref') as { inputEl?: HTMLInputElement } | null)?.inputEl instanceof HTMLInputElement,
    );
  });

  test('assigns the w-ref element to the component property', async ({ page }) => {
    const hasRef = await page.evaluate(() => {
      const host = document.querySelector('test-ref') as { inputEl?: HTMLInputElement } | null;
      return host?.inputEl instanceof HTMLInputElement;
    });

    expect(hasRef).toBe(true);
  });

  test('reads the input value through the ref', async ({ page }) => {
    await page.evaluate(() => {
      const host = document.querySelector('test-ref') as { inputEl: HTMLInputElement } | null;
      if (host?.inputEl) {
        host.inputEl.value = 'typed text';
      }
    });

    await page.locator('test-ref .read').click();
    await expect(page.locator('test-ref .display')).toHaveText('typed text');
  });

  test('focuses the input through the ref', async ({ page }) => {
    await page.locator('test-ref .focus').click();

    const focused = await page.evaluate(() => {
      const host = document.querySelector('test-ref') as (HTMLElement & {
        inputEl?: HTMLInputElement;
        shadowRoot: ShadowRoot | null;
      }) | null;
      // In shadow DOM, activeElement is on shadowRoot; in light DOM, on document
      const active = host?.shadowRoot?.activeElement ?? document.activeElement;
      return active === host?.inputEl;
    });

    expect(focused).toBe(true);
  });
});
}
