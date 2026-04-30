// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('attr fixture', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/attr/fixture.html');
    await page.waitForSelector('test-attr');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-attr');
      return el && (el as any).$ready === true;
    });
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

  test('boolean attr defaults to false', async ({ page }) => {
    const active = await page.evaluate(() => {
      return (document.querySelector('test-attr') as any).isActive;
    });
    expect(active).toBe(false);
  });

  test('boolean attr becomes true when attribute is set', async ({ page }) => {
    await page.evaluate(() => {
      document.querySelector('test-attr')!.setAttribute('is-active', '');
    });

    const active = await page.evaluate(() => {
      return (document.querySelector('test-attr') as any).isActive;
    });
    expect(active).toBe(true);
  });

  test('boolean attr becomes false when attribute is removed', async ({ page }) => {
    await page.evaluate(() => {
      const el = document.querySelector('test-attr')!;
      el.setAttribute('is-active', '');
      el.removeAttribute('is-active');
    });

    const active = await page.evaluate(() => {
      return (document.querySelector('test-attr') as any).isActive;
    });
    expect(active).toBe(false);
  });

  test('boolean attr updates template bindings', async ({ page }) => {
    // Initially false — no data-active attribute
    await expect(page.locator('test-attr .bool-target')).not.toHaveAttribute('data-active');

    // Set to true via property
    await page.evaluate(() => {
      (document.querySelector('test-attr') as any).isActive = true;
    });

    await expect(page.locator('test-attr .bool-target')).toHaveAttribute('data-active', '');
  });

  test('boolean attr sets checkbox checked property', async ({ page }) => {
    // Initially unchecked
    await expect(page.locator('test-attr .bool-check')).not.toBeChecked();

    // Set to true
    await page.evaluate(() => {
      (document.querySelector('test-attr') as any).isActive = true;
    });

    await expect(page.locator('test-attr .bool-check')).toBeChecked();

    // Set back to false
    await page.evaluate(() => {
      (document.querySelector('test-attr') as any).isActive = false;
    });

    await expect(page.locator('test-attr .bool-check')).not.toBeChecked();
  });

  test('mixed static+dynamic attribute renders correctly', async ({ page }) => {
    await expect(page.locator('test-attr .mixed')).toHaveAttribute('href', '/items/42');

    await page.evaluate(() => {
      (document.querySelector('test-attr') as any).itemId = '99';
    });

    await expect(page.locator('test-attr .mixed')).toHaveAttribute('href', '/items/99');
  });

  test('mixed attribute with prefix and suffix', async ({ page }) => {
    await expect(page.locator('test-attr .mixed-class')).toHaveAttribute('data-tag', 'prefix-demo-suffix');

    await page.evaluate(() => {
      (document.querySelector('test-attr') as any).tag = 'live';
    });

    await expect(page.locator('test-attr .mixed-class')).toHaveAttribute('data-tag', 'prefix-live-suffix');
  });
});
