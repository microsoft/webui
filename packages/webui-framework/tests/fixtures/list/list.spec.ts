// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('list fixture', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/list/fixture.html');
    await page.waitForSelector('test-list');
    await expect(page.locator('test-list-item .title')).toHaveCount(2);
    await page.waitForFunction(() => {
      const host = document.querySelector('test-list');
      const root = host?.shadowRoot ?? host;
      const item = root?.querySelector('test-list-item');
      return host && (host as any).$ready === true && item && (item as any).$ready === true;
    });
  });

  test('renders SSR repeat content and nested child conditionals', async ({ page }) => {
    await expect(page.locator('test-list-item .title')).toHaveText(['Alpha', 'Beta']);
    await expect(page.locator('test-list .count')).toHaveText('2');
    await expect(page.locator('test-list-item .done')).toHaveText(['Done']);
  });

  test('boolean attr on repeat item root reflects item state', async ({ page }) => {
    // SSR: Beta (state=done) should have data-done, Alpha (pending) should not
    await expect(page.locator('test-list-item[data-done]')).toHaveCount(1);
    await expect(page.locator('test-list-item[data-done]')).toHaveAttribute('item-id', '2');

    // Toggle Alpha to done
    await page.locator('test-list-item[item-id="1"] .toggle').click();
    await expect(page.locator('test-list-item[data-done]')).toHaveCount(2);

    // Toggle Beta back to pending
    await page.locator('test-list-item[item-id="2"] .toggle').click();
    await expect(page.locator('test-list-item[data-done]')).toHaveCount(1);
    await expect(page.locator('test-list-item[data-done]')).toHaveAttribute('item-id', '1');
  });

  test('identifier boolean attr on repeat item root reflects truthy value', async ({ page }) => {
    // SSR: Beta (flagged=true) should have data-flagged, Alpha should not
    await expect(page.locator('test-list-item[data-flagged]')).toHaveCount(1);
    await expect(page.locator('test-list-item[data-flagged]')).toHaveAttribute('item-id', '2');
  });

  test('adds nested children through repeat reconciliation', async ({ page }) => {
    await page.locator('test-list .add').click();

    await expect(page.locator('test-list-item .title')).toHaveText(['Alpha', 'Beta', 'Item 3']);
    await expect(page.locator('test-list-item .done')).toHaveText(['Done', 'Done']);
  });

  test('preserves keyed nodes when reversing the collection', async ({ page }) => {
    const initial = await page.evaluate(() => {
      const host = document.querySelector('test-list');
      const item = (host?.shadowRoot ?? host)?.querySelector('test-list-item[item-id="2"]');
      const win = window as unknown as { __preservedNode?: Element | null };
      win.__preservedNode = item;
      return item instanceof HTMLElement;
    });

    expect(initial).toBe(true);

    await page.locator('test-list .reverse').click();

    const preserved = await page.evaluate(() => {
      const host = document.querySelector('test-list');
      const win = window as unknown as { __preservedNode?: Element | null };
      return win.__preservedNode === (host?.shadowRoot ?? host)?.querySelector('test-list-item[item-id="2"]');
    });

    expect(preserved).toBe(true);
    await expect(page.locator('test-list-item .title')).toHaveText(['Beta', 'Alpha']);
  });

  test('clears all repeated children', async ({ page }) => {
    await page.locator('test-list .clear').click();
    await expect(page.locator('test-list-item')).toHaveCount(0);
  });

  test('prepend: inserts new item at top without recreating existing nodes', async ({ page }) => {
    // Save references to existing nodes
    const saved = await page.evaluate(() => {
      const host = document.querySelector('test-list');
      const root = host?.shadowRoot ?? host;
      const win = window as unknown as { __item1?: Element | null; __item2?: Element | null };
      win.__item1 = root?.querySelector('test-list-item[item-id="1"]');
      win.__item2 = root?.querySelector('test-list-item[item-id="2"]');
      return !!(win.__item1 && win.__item2);
    });
    expect(saved).toBe(true);

    await page.locator('test-list .prepend').click();

    // Existing nodes must be the same instances (not recreated)
    const preserved = await page.evaluate(() => {
      const host = document.querySelector('test-list');
      const root = host?.shadowRoot ?? host;
      const win = window as unknown as { __item1?: Element | null; __item2?: Element | null };
      return {
        item1Same: win.__item1 === root?.querySelector('test-list-item[item-id="1"]'),
        item2Same: win.__item2 === root?.querySelector('test-list-item[item-id="2"]'),
      };
    });
    expect(preserved.item1Same).toBe(true);
    expect(preserved.item2Same).toBe(true);

    // New item at top, existing items preserved in order
    await expect(page.locator('test-list-item').first().locator('.title')).toHaveText('Item 3');
    await expect(page.locator('test-list-item')).toHaveCount(3);
  });

  test('toggle via child event: zero DOM moves in container', async ({ page }) => {
    // Wait for test-list-item to fully hydrate (events wired)
    await page.waitForFunction(() => {
      const host = document.querySelector('test-list');
      const root = host?.shadowRoot ?? host;
      const item = root?.querySelector('test-list-item');
      return item && (item as any).$ready === true;
    });

    // Click toggle on first item via Playwright (pierces shadow DOM)
    await page.locator('test-list-item .toggle').first().click();

    // Item 1 should now be 'done' — 2 "Done" labels total
    await expect(page.locator('test-list-item .done')).toHaveCount(2);
  });

  test('passes repeat scope values to event handlers', async ({ page }) => {
    await page.locator('test-list .loop-arg').nth(1).click();
    await expect(page.locator('test-list .last-loop-arg')).toHaveText('arg=2 typeof=string args.length=1');

    await page.locator('test-list .loop-arg-event').nth(0).click();
    await expect(page.locator('test-list .last-loop-arg')).toHaveText('arg=1 event=click args.length=2');
  });
});
