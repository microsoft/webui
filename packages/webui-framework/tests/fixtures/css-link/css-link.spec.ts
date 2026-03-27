// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('css link fixture', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/css-link/fixture.html');
    await page.waitForSelector('test-link-host .spawn');
  });

  test('client-created components keep link stylesheets in compiled static html', async ({ page }) => {
    const hostHref = await page.locator('test-link-host').evaluate((host) =>
      host.shadowRoot?.querySelector('link[rel="stylesheet"]')?.getAttribute('href') ?? null,
    );
    expect(hostHref).toBe('/css-link/host.css');

    await expect.poll(async () => page.locator('test-link-host').evaluate((host) => {
      const label = host.shadowRoot?.querySelector('.host-label');
      return label instanceof HTMLElement ? getComputedStyle(label).color : null;
    })).toBe('rgb(34, 139, 34)');

    await page.locator('test-link-host .spawn').click();

    const childHref = await page.locator('test-link-host').evaluate((host) => {
      const child = host.shadowRoot?.querySelector('test-link-child');
      return child?.shadowRoot?.querySelector('link[rel="stylesheet"]')?.getAttribute('href') ?? null;
    });
    expect(childHref).toBe('/css-link/child.css');

    await expect.poll(async () => page.locator('test-link-host').evaluate((host) => {
      const child = host.shadowRoot?.querySelector('test-link-child');
      const label = child?.shadowRoot?.querySelector('.child-label');
      return label instanceof HTMLElement ? getComputedStyle(label).color : null;
    })).toBe('rgb(128, 0, 128)');
  });
});
