// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('ref fixture', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/ref/fixture.html');
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
      const active = host?.shadowRoot?.activeElement;
      return active === host?.inputEl;
    });

    expect(focused).toBe(true);
  });

  test('clears w-ref properties on destroy', async ({ page }) => {
    const result = await page.evaluate(() => {
      const host = document.querySelector('test-ref') as (HTMLElement & {
        $destroy(): void;
        inputEl?: HTMLInputElement;
      }) | null;
      const hadRef = host?.inputEl instanceof HTMLInputElement;
      host?.$destroy();
      return { hadRef, cleared: host?.inputEl === undefined };
    });

    expect(result).toEqual({ hadRef: true, cleared: true });
  });
});
