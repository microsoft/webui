// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('raw HTML in @attr inside <for> loop', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/raw-html-for-attr/fixture.html');
    await page.waitForSelector('test-raw-for');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-raw-for');
      return el && (el as any).$ready === true;
    });
  });

  test('SSR: {{{htmlContent}}} renders raw HTML from @attr', async ({ page }) => {
    // After SSR + hydration, the body should contain rendered HTML, not escaped tags
    const body1 = await page.locator('test-raw-item:first-of-type .item-body').innerHTML();
    expect(body1).toContain('<p>Hello <strong>World</strong></p>');
    // Ensure it does NOT contain escaped HTML like &lt;p&gt;
    expect(body1).not.toContain('&lt;');
  });

  test('SSR: plain {{name}} renders correctly alongside raw HTML', async ({ page }) => {
    await expect(page.locator('test-raw-item:first-of-type .item-name')).toHaveText('Email 1');
    await expect(page.locator('test-raw-item:nth-of-type(2) .item-name')).toHaveText('Email 2');
  });

  test('reactive update: new items render {{{htmlContent}}} as raw HTML', async ({ page }) => {
    // Reactively replace the items array — simulates SPA navigation
    await page.evaluate(() => {
      const el = document.querySelector('test-raw-for') as any;
      el.items = [
        { name: 'New 1', htmlContent: '<p>Updated <b>bold</b> text</p>' },
        { name: 'New 2', htmlContent: '<div class="custom"><span>Rich</span> content</div>' },
      ];
    });

    // Wait for reactive update to propagate
    await page.waitForTimeout(100);

    // Verify the new items render raw HTML, not escaped
    const body1 = await page.locator('test-raw-item:first-of-type .item-body').innerHTML();
    expect(body1).toContain('<p>Updated <b>bold</b> text</p>');
    expect(body1).not.toContain('&lt;');

    const body2 = await page.locator('test-raw-item:nth-of-type(2) .item-body').innerHTML();
    expect(body2).toContain('<div class="custom"><span>Rich</span> content</div>');
    expect(body2).not.toContain('&lt;');
  });

  test('reactive update: names update correctly alongside raw HTML', async ({ page }) => {
    await page.evaluate(() => {
      const el = document.querySelector('test-raw-for') as any;
      el.items = [
        { name: 'Alice', htmlContent: '<p>Message from Alice</p>' },
      ];
    });
    await page.waitForTimeout(100);
    await expect(page.locator('test-raw-item .item-name')).toHaveText('Alice');
  });
});
