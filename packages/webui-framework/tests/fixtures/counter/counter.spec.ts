// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

for (const mode of ['light', 'shadow'] as const) {
test.describe(`counter fixture [${mode} DOM]`, () => {
  test.beforeEach(async ({ page }) => {
    const file = mode === 'light' ? 'fixture.html' : 'fixture-shadow.html';
    await page.goto(`/counter/${file}`);
    await page.waitForSelector('test-counter');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-counter');
      return el && (el as any).$ready === true;
    });
  });

  test('renders SSR content', async ({ page }) => {
    await expect(page.locator('test-counter .label')).toHaveText('Clicks');
    await expect(page.locator('test-counter .count')).toHaveText('0');
    await expect(page.locator('test-counter .doubled')).toHaveText('0');
  });

  test('updates count through click handlers', async ({ page }) => {
    await page.locator('test-counter .inc').click();
    await page.locator('test-counter .inc').click();
    await page.locator('test-counter .dec').click();

    await expect(page.locator('test-counter .count')).toHaveText('1');
  });

  test('updates derived @observable reactively', async ({ page }) => {
    await page.locator('test-counter .inc').click();
    await page.locator('test-counter .inc').click();

    await expect(page.locator('test-counter .doubled')).toHaveText('4');
  });

  test('updates @attr labels reactively', async ({ page }) => {
    await page.evaluate(() => {
      document.querySelector('test-counter')?.setAttribute('label', 'Count');
    });

    await expect(page.locator('test-counter .label')).toHaveText('Count');
  });
});
}
