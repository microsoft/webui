// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Regression test: complex property bindings (:prop) propagate parent
 * observable changes to child <for> loops.
 *
 * When a parent's @observable array is replaced (e.g. via setInitialState
 * during SPA navigation), the complex binding `:items="{{items}}"` must
 * push the new array to the child, causing the child's <for> loop to
 * re-render with the updated data.
 */

import { expect, test } from '@playwright/test';

test.describe('complex-prop: parent array changes propagate to child for-loop', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/complex-prop/fixture.html');
    await page.waitForFunction(() => {
      const el = document.querySelector('#host') as any;
      return el && el.$ready === true;
    });
  });

  test('initial items render in child for-loop', async ({ page }) => {
    const items = await page.evaluate(() => {
      const host = document.querySelector('#host') as any;
      const list = host?.shadowRoot?.querySelector('test-item-list');
      const lis = list?.shadowRoot?.querySelectorAll('.item');
      return Array.from(lis ?? []).map((li: any) => li.textContent);
    });

    expect(items).toEqual(['Alpha', 'Beta', 'Gamma']);
  });

  test('replacing parent items updates child for-loop', async ({ page }) => {
    await page.evaluate(() => {
      const host = document.querySelector('#host') as any;
      host.replaceItems();
    });

    // Wait for microtask flush
    await page.waitForFunction(() => {
      const host = document.querySelector('#host') as any;
      const list = host?.shadowRoot?.querySelector('test-item-list');
      const lis = list?.shadowRoot?.querySelectorAll('.item');
      return lis?.length === 2;
    }, null, { timeout: 2000 });

    const items = await page.evaluate(() => {
      const host = document.querySelector('#host') as any;
      const list = host?.shadowRoot?.querySelector('test-item-list');
      const lis = list?.shadowRoot?.querySelectorAll('.item');
      return Array.from(lis ?? []).map((li: any) => li.textContent);
    });

    expect(items).toEqual(['One', 'Two']);
  });

  test('clearing parent items empties child for-loop', async ({ page }) => {
    await page.evaluate(() => {
      const host = document.querySelector('#host') as any;
      host.clearItems();
    });

    await page.waitForFunction(() => {
      const host = document.querySelector('#host') as any;
      const list = host?.shadowRoot?.querySelector('test-item-list');
      const lis = list?.shadowRoot?.querySelectorAll('.item');
      return lis?.length === 0;
    }, null, { timeout: 2000 });

    const count = await page.evaluate(() => {
      const host = document.querySelector('#host') as any;
      const list = host?.shadowRoot?.querySelector('test-item-list');
      return list?.shadowRoot?.querySelectorAll('.item')?.length;
    });

    expect(count).toBe(0);
  });

  test('setInitialState on parent propagates to child for-loop', async ({ page }) => {
    // Simulate what the router does during SPA navigation
    await page.evaluate(() => {
      const host = document.querySelector('#host') as any;
      host.setInitialState({ items: [{ name: 'X' }, { name: 'Y' }] });
    });

    await page.waitForFunction(() => {
      const host = document.querySelector('#host') as any;
      const list = host?.shadowRoot?.querySelector('test-item-list');
      const lis = list?.shadowRoot?.querySelectorAll('.item');
      return lis?.length === 2;
    }, null, { timeout: 2000 });

    const items = await page.evaluate(() => {
      const host = document.querySelector('#host') as any;
      const list = host?.shadowRoot?.querySelector('test-item-list');
      const lis = list?.shadowRoot?.querySelectorAll('.item');
      return Array.from(lis ?? []).map((li: any) => li.textContent);
    });

    expect(items).toEqual(['X', 'Y']);
  });

  test('setInitialState propagates to child synchronously (no microtask needed)', async ({ page }) => {
    // The critical case: after setInitialState, the child DOM must be
    // updated synchronously — no microtask wait. This matters for view
    // transitions which snapshot the DOM right after the sync callback.
    const result = await page.evaluate(() => {
      const host = document.querySelector('#host') as any;
      host.setInitialState({ items: [{ name: 'Sync1' }, { name: 'Sync2' }, { name: 'Sync3' }] });

      // Check IMMEDIATELY — no await, no microtask, no setTimeout
      const list = host?.shadowRoot?.querySelector('test-item-list');
      const lis = list?.shadowRoot?.querySelectorAll('.item');
      return {
        count: lis?.length,
        items: Array.from(lis ?? []).map((li: any) => li.textContent),
      };
    });

    expect(result.count).toBe(3);
    expect(result.items).toEqual(['Sync1', 'Sync2', 'Sync3']);
  });
});
