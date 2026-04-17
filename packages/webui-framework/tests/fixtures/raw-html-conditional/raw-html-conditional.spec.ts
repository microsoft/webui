// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('raw HTML inside conditional', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/raw-html-conditional/fixture.html');
    await page.waitForSelector('test-raw-html');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-raw-html');
      return el && (el as any).$ready === true;
    });
  });

  test('header retains text after hydration when sibling has {{{raw}}}', async ({ page }) => {
    // After SSR + hydration, the structured header must still contain "Alice",
    // not the raw HTML body content.
    await expect(page.locator('test-raw-html .header .name')).toHaveText('Alice');
  });

  test('raw HTML body renders in its own container', async ({ page }) => {
    const bodyHtml = await page.locator('test-raw-html .body').innerHTML();
    expect(bodyHtml).toContain('<p>Hello</p>');
  });

  test('header does not contain raw HTML body content', async ({ page }) => {
    const headerHtml = await page.locator('test-raw-html .header').innerHTML();
    expect(headerHtml).not.toContain('Hello');
  });
});
