// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('attr fixture', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/attr/fixture.html');
    await page.waitForSelector('test-attr');
  });

  test('renders attribute-backed SSR text', async ({ page }) => {
    await expect(page.locator('test-attr .label')).toHaveText('Status');
    await expect(page.locator('test-attr .display')).toHaveText('Ready');
  });

  test('updates default attribute names reactively', async ({ page }) => {
    await page.evaluate(() => {
      document.querySelector('test-attr')?.setAttribute('label', 'Mode');
    });

    await expect(page.locator('test-attr .label')).toHaveText('Mode');
  });

  test('updates custom attribute names reactively', async ({ page }) => {
    await page.evaluate(() => {
      document.querySelector('test-attr')?.setAttribute('display-value', 'Paused');
    });

    await expect(page.locator('test-attr .display')).toHaveText('Paused');
  });

  test('reacts to direct property updates', async ({ page }) => {
    await page.evaluate(() => {
      const host = document.querySelector('test-attr') as { label: string; displayValue: string } | null;
      if (host) {
        host.label = 'Phase';
        host.displayValue = 'Running';
      }
    });

    await expect(page.locator('test-attr .label')).toHaveText('Phase');
    await expect(page.locator('test-attr .display')).toHaveText('Running');
  });

  test('keeps event markers from hijacking attr hydration targets', async ({ page }) => {
    await page.evaluate(() => {
      const host = document.querySelector('test-attr') as { ctaHref: string } | null;
      if (host) {
        host.ctaHref = '/cart';
      }
    });

    await expect(page.locator('test-attr .cta')).toHaveAttribute('href', '/cart');
    await expect(page.locator('test-attr .logo')).toHaveAttribute('href', '/');
  });
});
