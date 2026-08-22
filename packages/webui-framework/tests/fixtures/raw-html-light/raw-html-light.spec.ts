// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test('light DOM raw updates preserve static and conditional siblings', async ({ page }) => {
  await page.goto('/raw-html-light/fixture.html');
  await page.waitForFunction(() => {
    const element = document.querySelector('test-raw-html-light');
    return element && (element as unknown as { $ready?: boolean }).$ready === true;
  });

  const host = page.locator('test-raw-html-light');
  expect(await host.evaluate((element) => element.shadowRoot === null)).toBe(true);

  const body = host.locator('.body');
  await expect(body.locator(':scope > *')).toHaveClass([
    'before',
    'raw-initial',
    'conditional',
    'after',
  ]);

  await page.evaluate(() => {
    const host = document.querySelector('test-raw-html-light') as HTMLElement & {
      rawHtml: string;
    };
    host.rawHtml = '<b class="raw-first">One</b><i class="raw-second">Two</i>';
  });
  await expect(body.locator(':scope > *')).toHaveClass([
    'before',
    'raw-first',
    'raw-second',
    'conditional',
    'after',
  ]);

  await page.evaluate(() => {
    const host = document.querySelector('test-raw-html-light') as HTMLElement & {
      rawHtml: string;
    };
    host.rawHtml = '';
  });
  await expect(body.locator(':scope > *')).toHaveClass([
    'before',
    'conditional',
    'after',
  ]);

  await page.evaluate(() => {
    const host = document.querySelector('test-raw-html-light') as HTMLElement & {
      rawHtml: string;
    };
    host.rawHtml = '<strong class="raw-restored">Restored</strong>';
  });
  await expect(body.locator(':scope > *')).toHaveClass([
    'before',
    'raw-restored',
    'conditional',
    'after',
  ]);

  await page.evaluate(() => {
    const host = document.querySelector('test-raw-html-light') as HTMLElement & {
      showConditional: boolean;
    };
    host.showConditional = false;
  });
  await expect(body.locator(':scope > *')).toHaveClass([
    'before',
    'raw-restored',
    'after',
  ]);

  await page.evaluate(() => {
    const host = document.querySelector('test-raw-html-light') as HTMLElement & {
      showConditional: boolean;
    };
    host.showConditional = true;
  });
  await expect(body.locator(':scope > *')).toHaveClass([
    'before',
    'raw-restored',
    'conditional',
    'after',
  ]);
});

test('raw HTML parses in the live table context', async ({ page }) => {
  await page.goto('/raw-html-light/fixture.html');
  await page.waitForFunction(() => {
    const element = document.querySelector('test-raw-html-light');
    return element && (element as unknown as { $ready?: boolean }).$ready === true;
  });

  const rows = page.locator('test-raw-html-light .rows');
  await expect(rows.locator(':scope > tr')).toHaveClass([
    'row-before',
    'raw-row-initial',
    'row-after',
  ]);

  await page.evaluate(() => {
    const host = document.querySelector('test-raw-html-light') as HTMLElement & {
      rowsHtml: string;
    };
    host.rowsHtml =
      '<tr class="raw-row-a"><td>A</td></tr><tr class="raw-row-b"><td>B</td></tr>';
  });

  await expect(rows.locator(':scope > tr')).toHaveClass([
    'row-before',
    'raw-row-a',
    'raw-row-b',
    'row-after',
  ]);
  await expect(rows.locator(':scope > tr').nth(1).locator('td')).toHaveText('A');
  await expect(rows.locator(':scope > tr').nth(2).locator('td')).toHaveText('B');
});
