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
    await expect(host.locator('.conditional')).not.toHaveAttribute('w-ref');
    expect(await host.evaluate(element =>
      (element as TestRef).conditionalEl === element.shadowRoot?.querySelector('.conditional'),
    )).toBe(true);
    await host.locator('.toggle').click();
    await expect(host.locator('.conditional')).toHaveCount(0);
    expect(await host.evaluate(element => (element as TestRef).conditionalEl)).toBeUndefined();
    expect(await host.evaluate(element => (element as TestRef).inputEl instanceof HTMLInputElement)).toBe(true);
  });

  test('removing a view preserves a newer assignment to its owner ref', async ({ page }) => {
    const host = page.locator('test-ref');
    await host.locator('.toggle').click();
    await expect(host.locator('.conditional')).toHaveCount(1);
    await host.evaluate(element => {
      const owner = element as TestRef;
      owner.conditionalEl = owner.inputEl;
    });
    await host.locator('.toggle').click();
    await expect(host.locator('.conditional')).toHaveCount(0);
    expect(await host.evaluate(element => {
      const owner = element as TestRef;
      return owner.conditionalEl === owner.inputEl;
    })).toBe(true);
  });
});
