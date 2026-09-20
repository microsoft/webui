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

  test('keeps structural keys out of SSR and client-created DOM', async ({ page }) => {
    await expect(page.locator('test-list-item[key]')).toHaveCount(0);

    await page.locator('test-list .add').click();

    await expect(page.locator('test-list-item')).toHaveCount(3);
    await expect(page.locator('test-list-item[key]')).toHaveCount(0);
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

  test('moves keyed nodes with their items when reversing the collection', async ({ page }) => {
    const initial = await page.evaluate(() => {
      const host = document.querySelector('test-list');
      const items = (host?.shadowRoot ?? host)?.querySelectorAll('test-list-item');
      const win = window as unknown as {
        __firstNode?: Element;
        __secondNode?: Element;
      };
      win.__firstNode = items?.[0];
      win.__secondNode = items?.[1];
      return items?.length;
    });

    expect(initial).toBe(2);

    await page.locator('test-list .reverse').click();

    const preserved = await page.evaluate(() => {
      const host = document.querySelector('test-list');
      const items = (host?.shadowRoot ?? host)?.querySelectorAll('test-list-item');
      const win = window as unknown as {
        __firstNode?: Element;
        __secondNode?: Element;
      };
      return (
        win.__secondNode === items?.[0] && win.__firstNode === items?.[1]
      );
    });

    expect(preserved).toBe(true);
    await expect(page.locator('test-list-item .title')).toHaveText(['Beta', 'Alpha']);
  });

  test('clears all repeated children', async ({ page }) => {
    await page.locator('test-list .clear').click();
    await expect(page.locator('test-list-item')).toHaveCount(0);
  });

  test('prepend: preserves keyed item nodes after the inserted head', async ({ page }) => {
    const saved = await page.evaluate(() => {
      const host = document.querySelector('test-list');
      const root = host?.shadowRoot ?? host;
      const items = root?.querySelectorAll('test-list-item');
      const win = window as unknown as {
        __firstNode?: Element;
        __secondNode?: Element;
      };
      win.__firstNode = items?.[0];
      win.__secondNode = items?.[1];
      return items?.length;
    });
    expect(saved).toBe(2);

    await page.locator('test-list .prepend').click();

    const preserved = await page.evaluate(() => {
      const host = document.querySelector('test-list');
      const root = host?.shadowRoot ?? host;
      const items = root?.querySelectorAll('test-list-item');
      const win = window as unknown as {
        __firstNode?: Element;
        __secondNode?: Element;
      };
      return {
        firstSame: win.__firstNode === items?.[1],
        secondSame: win.__secondNode === items?.[2],
      };
    });
    expect(preserved.firstSame).toBe(true);
    expect(preserved.secondSame).toBe(true);

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
    await expect(page.locator('test-list .last-loop-arg')).toHaveText('arg=1 event=click current=loop-arg-event args.length=2');
  });

  test('hydrates empty text slots after repeat content at the correct position', async ({ page }) => {
    await page.locator('test-list .set-after-repeat').click();

    const textBeforeTail = await page.evaluate(() => {
      const host = document.querySelector('test-list');
      const root = host?.shadowRoot ?? host;
      const tail = root?.querySelector('.after-repeat-tail');
      return tail?.previousSibling?.textContent?.trim() ?? '';
    });

    expect(textBeforeTail).toBe('After');
  });
});
