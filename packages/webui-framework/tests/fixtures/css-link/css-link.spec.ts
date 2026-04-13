// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('css link fixture', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/css-link/fixture.html');
    await page.waitForSelector('test-link-host .spawn');
  });

  test('client-created components apply link stylesheet styles', async ({ page }) => {
    // Host component should have green text from host.css
    await expect.poll(async () => page.locator('test-link-host').evaluate((host) => {
      const label = (host.shadowRoot ?? host).querySelector('.host-label');
      return label instanceof HTMLElement ? getComputedStyle(label).color : null;
    })).toBe('rgb(34, 139, 34)');

    // Spawn a client-created child component
    await page.locator('test-link-host .spawn').click();

    // Child component should have purple text from child.css
    // (styles applied via adoptedStyleSheets or <link>, either is correct)
    await expect.poll(async () => page.locator('test-link-host').evaluate((host) => {
      const child = (host.shadowRoot ?? host).querySelector('test-link-child');
      const label = (child?.shadowRoot ?? child)?.querySelector('.child-label');
      return label instanceof HTMLElement ? getComputedStyle(label).color : null;
    })).toBe('rgb(128, 0, 128)');
  });
});
