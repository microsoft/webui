// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('multi repeat fixture', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/multi-repeat/fixture.html');
    await page.waitForSelector('test-multi-repeat');
  });

  test('second list conditionals update when items change', async ({ page }) => {
    const errors: string[] = [];
    page.on('pageerror', (err) => errors.push(err.message));

    // Verify initial SSR state — Alpha is <p>, Beta is <a> in both lists
    const initial = await page.evaluate(() => {
      const host = document.querySelector('test-multi-repeat') as any;
      const sr = host?.shadowRoot;
      const listA = sr?.querySelector('.list-a');
      const listB = sr?.querySelector('.list-b');
      return {
        listALinks: listA?.querySelectorAll('a.link')?.length,
        listBLinks: listB?.querySelectorAll('a.link')?.length,
        listACurrents: listA?.querySelectorAll('p.current')?.length,
        listBCurrents: listB?.querySelectorAll('p.current')?.length,
      };
    });

    expect(initial.listALinks).toBe(1);
    expect(initial.listBLinks).toBe(1);
    expect(initial.listACurrents).toBe(1);
    expect(initial.listBCurrents).toBe(1);

    // Update items — switch active from Alpha to Beta
    const after = await page.evaluate(() => {
      const host = document.querySelector('test-multi-repeat') as any;
      host.items = [
        { title: 'Alpha', href: '/alpha', active: 'false' },
        { title: 'Beta', href: '/beta', active: 'true' },
      ];

      const sr = host.shadowRoot;
      const listB = sr?.querySelector('.list-b');
      const betaCurrent = listB?.querySelector('p.current');
      const alphaLink = listB?.querySelector('a.link');
      return {
        betaIsCurrent: betaCurrent?.textContent?.trim(),
        alphaIsLink: alphaLink?.textContent?.trim(),
        alphaHref: alphaLink?.getAttribute('href'),
      };
    });

    // The second list's conditionals must have toggled
    expect(after.betaIsCurrent).toBe('Beta');
    expect(after.alphaIsLink).toBe('Alpha');
    expect(after.alphaHref).toBe('/alpha');
    expect(errors).toEqual([]);
  });
});
