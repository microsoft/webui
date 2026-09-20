// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('unkeyed repeat reconciliation', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/repeat-append/fixture.html');
    await page.waitForFunction(() => {
      const parent = document.querySelector('test-repeat-parent');
      const children = parent?.shadowRoot?.querySelectorAll('test-repeat-child');
      return (
        parent
        && (parent as unknown as { $ready?: boolean }).$ready === true
        && children?.length === 5
        && Array.from(children).every(
          (child) => (child as unknown as { $ready?: boolean }).$ready === true,
        )
      );
    });
  });

  test('hydrates SSR children and appends at the tail', async ({ page }) => {
    const labels = page.locator('test-repeat-child .label');
    await expect(labels).toHaveText([
      'Item 1',
      'Item 2',
      'Item 3',
      'Item 4',
      'Item 5',
    ]);

    await page.locator('test-repeat-parent .add').click();

    await expect(labels).toHaveText([
      'Item 1',
      'Item 2',
      'Item 3',
      'Item 4',
      'Item 5',
      'Item 6',
    ]);
  });

  test('reuses blocks by position when prepending', async ({ page }) => {
    await page.evaluate(() => {
      const parent = document.querySelector('test-repeat-parent');
      const children = parent?.shadowRoot?.querySelectorAll('test-repeat-child');
      (window as unknown as { __unkeyedNodes?: Element[] }).__unkeyedNodes =
        Array.from(children ?? []);
    });

    await page.locator('test-repeat-parent .prepend').click();

    const result = await page.evaluate(() => {
      const parent = document.querySelector('test-repeat-parent');
      const children = Array.from(
        parent?.shadowRoot?.querySelectorAll('test-repeat-child') ?? [],
      );
      const previous =
        (window as unknown as { __unkeyedNodes?: Element[] }).__unkeyedNodes ?? [];
      return {
        count: children.length,
        reusedByPosition: previous.every((node, index) => node === children[index]),
      };
    });

    expect(result).toEqual({ count: 6, reusedByPosition: true });
    await expect(page.locator('test-repeat-child .label')).toHaveText([
      'Item 6',
      'Item 1',
      'Item 2',
      'Item 3',
      'Item 4',
      'Item 5',
    ]);
  });

  test('updates nested children through middle insertion and deletion', async ({ page }) => {
    const labels = page.locator('test-repeat-child .label');
    await page.locator('test-repeat-parent .insert-middle').click();

    await expect(labels).toHaveText([
      'Item 1',
      'Item 2',
      'Item 6',
      'Item 3',
      'Item 4',
      'Item 5',
    ]);

    await page.locator('test-repeat-parent .remove-middle').click();

    await expect(labels).toHaveText([
      'Item 1',
      'Item 2',
      'Item 3',
      'Item 4',
      'Item 5',
    ]);
  });
});
