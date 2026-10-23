// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';
import type { TestRef } from './element.js';

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

  test('ordinary conditionals use compiled root refs and release them on removal', async ({ page }) => {
    const host = page.locator('test-ref');
    await host.locator('.toggle').click();
    await expect(host.locator('.conditional')).toHaveCount(1);
    expect(await host.evaluate(element =>
      (element as TestRef).conditionalEl === element.shadowRoot?.querySelector('.conditional'),
    )).toBe(true);
    await host.locator('.toggle').click();
    await expect(host.locator('.conditional')).toHaveCount(0);
    expect(await host.evaluate(element => (element as TestRef).conditionalEl)).toBeUndefined();
    expect(await host.evaluate(element => (element as TestRef).inputEl instanceof HTMLInputElement)).toBe(true);
  });
});
