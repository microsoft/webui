// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('light DOM structural bindings', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/light-dom-structural/fixture.html');
    await page.waitForFunction(() => {
      const element = document.querySelector('test-light-dom-structural');
      return element && (element as unknown as { $ready?: boolean }).$ready === true;
    });
  });

  test('hydrates and updates text-only conditional and repeat blocks', async ({ page }) => {
    const host = page.locator('test-light-dom-structural');

    await expect(host).toHaveText('beforeconditionalABafter');
    expect(await host.evaluate((element) => element.shadowRoot)).toBeNull();

    await host.evaluate((element) => {
      const target = element as HTMLElement & {
        conditionalText: string;
        items: string[];
        show: boolean;
      };
      target.show = false;
      target.conditionalText = 'updated';
      target.items = ['C', 'D'];
    });
    await expect(host).toHaveText('beforeCDafter');

    await host.evaluate((element) => {
      (element as HTMLElement & { show: boolean }).show = true;
    });
    await expect(host).toHaveText('beforeupdatedCDafter');
  });

  test('creates the same text-only structure entirely on the client', async ({ page }) => {
    const result = await page.evaluate(() => {
      const element = document.createElement('test-light-dom-structural') as HTMLElement & {
        $ready?: boolean;
      };
      element.id = 'client-light-structural';
      document.body.appendChild(element);
      return {
        hasShadow: element.shadowRoot !== null,
        ready: element.$ready === true,
        text: element.textContent,
      };
    });

    expect(result).toEqual({
      hasShadow: false,
      ready: true,
      text: 'beforeclientXafter',
    });
  });
});
