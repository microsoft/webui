// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

/**
 * Root bindings on a host-interactive component, driven by trusted browser
 * input rather than synthetic `dispatchEvent` calls.
 */
test.describe('host-interactive fixture', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/host-interactive/fixture.html');
    await page.waitForSelector('test-host-toggle');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-host-toggle');
      return el && (el as any).$ready === true;
    });
  });

  test('a real click on the host fires the root @click binding', async ({ page }) => {
    await page.locator('test-host-toggle').click();

    await expect(page.locator('test-host-toggle .count')).toHaveText('1');
    await expect(page.locator('test-host-toggle .indicator')).toHaveText('on');
  });

  test('the click really did target the host, not shadow content', async ({ page }) => {
    // Guards the fixture itself: `target` is retargeted to the host either way,
    // so assert on the un-retargeted origin. If the shadow indicator ever became
    // clickable, this catches it and the other tests stop silently degrading
    // into the ordinary shadow-tree case.
    await expect(page.locator('test-host-toggle')).toHaveAttribute('tabindex', '0');
    await page.locator('test-host-toggle').click();

    const seen = await page.evaluate(() => {
      const el = document.querySelector('test-host-toggle') as any;
      return { target: el?.lastTarget, origin: el?.lastOrigin };
    });
    expect(seen.origin).toBe('TEST-HOST-TOGGLE');
    expect(seen.target).toBe('TEST-HOST-TOGGLE');
  });

  test('a real keystroke on the focused host fires the root @keydown binding', async ({ page }) => {
    await page.locator('test-host-toggle').focus();
    await page.keyboard.press('Enter');

    await expect(page.locator('test-host-toggle .count')).toHaveText('1');
    await expect(page.locator('test-host-toggle .indicator')).toHaveText('on');

    const target = await page.evaluate(() => {
      const el = document.querySelector('test-host-toggle') as any;
      return el?.lastTarget;
    });
    expect(target).toBe('TEST-HOST-TOGGLE');
  });

  test('Space activates the host control as well', async ({ page }) => {
    await page.locator('test-host-toggle').focus();
    await page.keyboard.press(' ');

    await expect(page.locator('test-host-toggle .count')).toHaveText('1');
  });

  test('each interaction activates exactly once', async ({ page }) => {
    // A double fire would toggle and immediately untoggle, leaving the label
    // unchanged while the counter advanced by two.
    const toggle = page.locator('test-host-toggle');

    await toggle.click();
    await expect(page.locator('test-host-toggle .count')).toHaveText('1');
    await expect(page.locator('test-host-toggle .indicator')).toHaveText('on');

    await toggle.click();
    await expect(page.locator('test-host-toggle .count')).toHaveText('2');
    await expect(page.locator('test-host-toggle .indicator')).toHaveText('off');

    await toggle.focus();
    await page.keyboard.press('Enter');
    await expect(page.locator('test-host-toggle .count')).toHaveText('3');
    await expect(page.locator('test-host-toggle .indicator')).toHaveText('on');
  });
});
