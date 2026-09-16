// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

const aliasValues: ReadonlyArray<readonly [string, boolean | string]> = [
  ['inherited-old', true],
  ['inherited-new', ''],
  ['inherited-old-first', ''],
  ['inherited-new-first', true],
  ['stacked-first', true],
  ['stacked-second', ''],
  ['stacked-first-order', ''],
  ['stacked-second-order', true],
];

test('literal attributes produce visible, named native dialogs without JavaScript', async ({ browser, baseURL }) => {
  const context = await browser.newContext({ javaScriptEnabled: false, baseURL });
  const page = await context.newPage();
  try {
    await page.goto('/ssr-attributes/fixture.html');
    const dialog = page.locator('#literal dialog');
    await expect(dialog).toHaveJSProperty('open', true);
    await expect(dialog).toBeVisible();
    await expect(dialog).toHaveAccessibleName('Canvas information');
    await expect(dialog).toHaveAccessibleDescription('Dialog description');
    await expect(dialog.locator('h2')).toHaveText('Projected title');
    await expect(page.locator('#literal > p')).toBeVisible();
    for (const id of ['empty', 'false-text', 'dynamic']) {
      await expect(page.locator(`#${id} dialog`)).toHaveJSProperty('open', true);
    }
    for (const id of ['absent', 'closed']) {
      await expect(page.locator(`#${id} dialog`)).toHaveJSProperty('open', false);
    }
    await expect(page.locator('#empty dialog')).toHaveAttribute('aria-label', '');
    await expect(page.locator('#absent dialog')).toHaveAttribute('aria-label', 'Default name');
    for (const [id, value] of aliasValues) {
      await expect(page.locator(`#${id} dialog`)).toHaveJSProperty('open', Boolean(value));
    }
  } finally {
    await context.close();
  }
});

test('hydration adopts the SSR dialog and promotes it to the modal top layer', async ({ page }) => {
  let release!: () => void;
  const ready = new Promise<void>(resolve => { release = resolve; });
  await page.route('**/dist/ssr-attributes/element.js', async route => {
    await ready;
    await route.continue();
  });
  try {
    await page.goto('/ssr-attributes/fixture.html', { waitUntil: 'commit' });
    const dialog = page.locator('#literal dialog');
    await expect(dialog).toBeVisible();
    await expect(dialog).toHaveAccessibleName('Canvas information');
    const original = await dialog.elementHandle();
    expect(original).not.toBeNull();
    release();
    await page.waitForFunction(() => {
      const host = document.querySelector<HTMLElement & { $ready?: boolean }>('#literal');
      return host?.$ready === true;
    });
    expect(await original!.evaluate(node => node === document.querySelector('#literal')?.shadowRoot?.querySelector('dialog'))).toBe(true);
    await expect(dialog).toHaveJSProperty('open', true);
    expect(await dialog.evaluate(node => node.matches(':modal'))).toBe(true);
    await expect(dialog).toHaveAccessibleName('Canvas information');
    await expect(dialog).toHaveAccessibleDescription('Dialog description');
    await expect(dialog.locator('h2')).toHaveText('Projected title');
    await expect(page.locator('#absent dialog')).toHaveJSProperty('open', false);
    for (const [id, value] of aliasValues) {
      await expect(page.locator(`#${id}`)).toHaveJSProperty('expanded', value);
      await expect(page.locator(`#${id} dialog`)).toHaveJSProperty('open', Boolean(value));
    }
  } finally {
    release();
  }
});
