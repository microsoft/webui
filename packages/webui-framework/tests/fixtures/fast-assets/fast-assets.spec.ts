// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

for (const override of [false, true]) {
  const base = override ? '/override/' : '/static/';
  test(`FAST 3 SSR and client-created Link styles use ${base}`, async ({ page }) => {
    const errors: string[] = [];
    page.on('pageerror', error => errors.push(error.message));
    const response = await page.goto(`/fast-assets/nested/?${override ? 'override' : ''}`);
    expect(await response!.text()).toContain('data-webui-stylesheet');
    await expect(page.locator('review-fast-card button')).toHaveCSS('color', 'rgb(17, 81, 139)');
    await page.locator('review-fast-card button').click();
    await expect(page.locator('review-fast-card button')).toHaveText('2');
    await page.evaluate(() => document.body.appendChild(document.createElement('review-fast-card')));
    await expect(page.locator('review-fast-card button').last()).toHaveCSS('color', 'rgb(17, 81, 139)');
    expect(await page.locator('review-fast-card').last().evaluate((element, base) => {
      const root = element.shadowRoot!;
      return {
        compiler: root.querySelector<HTMLLinkElement>('link[data-webui-stylesheet]')!.href,
        authored: root.querySelector<HTMLLinkElement>('link:not([data-webui-stylesheet])')!.href,
      };
    }, base)).toEqual({
      compiler: new URL(`${base}review-fast-card.css`, page.url()).href,
      authored: new URL('authored.css', page.url()).href,
    });
    expect(errors).toEqual([]);
  });
  for (const version of ['2', '3']) for (const partial of [false, true]) {
    test(`FAST ${version} ${partial ? 'partial' : 'resource'} templates resolve ${base} before FAST client creation`, async ({ page }) => {
      await page.goto(`/fast-assets/nested/?empty&version=${version}${override ? '&override' : ''}`);
      await page.waitForFunction(() => 'mountFastCard' in window);
      expect(await page.evaluate(() => window.__webui?.assetBase)).toBeUndefined();
      await page.evaluate(partial => (window as unknown as {
        mountFastCard(partial: boolean): Promise<void>;
      }).mountFastCard(partial), partial);
      await expect(page.locator('review-fast-card button')).toHaveCSS('color', 'rgb(17, 81, 139)');
      expect(await page.locator('review-fast-card').evaluate(element =>
        element.shadowRoot!.querySelector<HTMLLinkElement>('link[data-webui-stylesheet]')!.href,
      )).toBe(new URL(`${base}review-fast-card.css`, page.url()).href);
      await page.locator('review-fast-card button').click();
      await expect(page.locator('review-fast-card button')).toHaveText('2');
    });
  }
}
