// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Regression test: complex property bindings (:prop) propagate parent
 * observable changes to child <for> loops.
 *
 * When a parent's @observable array is replaced (e.g. via setState
 * during SPA navigation), the complex binding `:items="{{sourceItems}}"` must
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

  test('child-first hydration receives the renamed parent property', async ({ page }) => {
    const result = await page.evaluate(() => {
      const host = document.querySelector('#host') as any;
      const list = host?.shadowRoot?.querySelector('test-item-list') as any;
      return {
        items: list?.items,
        shadowsAccessor: Object.hasOwn(list, 'items'),
      };
    });

    expect(result.items).toEqual([
      { name: 'Alpha' },
      { name: 'Beta' },
      { name: 'Gamma' },
    ]);
    expect(result.shadowsAccessor).toBe(false);
  });

  test('waits for a delayed custom-element accessor before assigning a property', async ({
    page,
  }) => {
    await page.waitForFunction(() => {
      const host = document.querySelector('#host') as any;
      const child = host?.shadowRoot?.querySelector(
        'test-delayed-prop-child',
      ) as any;
      return (
        customElements.get('test-delayed-prop-child') !== undefined &&
        child?.$ready === true
      );
    });
    const result = await page.evaluate(() => {
      const host = document.querySelector('#host') as any;
      const child = host?.shadowRoot?.querySelector(
        'test-delayed-prop-child',
      ) as any;
      return {
        items: child?.items,
        rendered: Array.from(
          child?.shadowRoot?.querySelectorAll('.delayed-item') ?? [],
        ).map((item: any) => item.textContent),
        shadowsAccessor: Object.hasOwn(child, 'items'),
      };
    });

    expect(result.items).toEqual([
      { name: 'Late Alpha' },
      { name: 'Late Beta' },
    ]);
    expect(result.rendered).toEqual(['Late Alpha', 'Late Beta']);
    expect(result.shadowsAccessor).toBe(false);
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

  test('setState on parent propagates to child for-loop', async ({ page }) => {
    // Simulate what the router does during SPA navigation
    await page.evaluate(() => {
      const host = document.querySelector('#host') as any;
      host.setState({ sourceItems: [{ name: 'X' }, { name: 'Y' }] });
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

  test('setState propagates to child synchronously (no microtask needed)', async ({ page }) => {
    // The critical case: after setState, the child DOM must be
    // updated synchronously — no microtask wait. This matters for view
    // transitions which snapshot the DOM right after the sync callback.
    const result = await page.evaluate(() => {
      const host = document.querySelector('#host') as any;
      host.setState({
        sourceItems: [{ name: 'Sync1' }, { name: 'Sync2' }, { name: 'Sync3' }],
      });

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

test.describe('complex-prop: parent-first SSR hydration', () => {
  test('queues renamed parent state until the child upgrades', async ({ page }) => {
    await page.goto('/complex-prop/fixture.html?definitionOrder=parent-first');
    await page.waitForFunction(() => {
      const el = document.querySelector('#host') as any;
      return el && el.$ready === true;
    });

    const result = await page.evaluate(() => {
      const host = document.querySelector('#host') as any;
      const list = host?.shadowRoot?.querySelector('test-item-list') as any;
      return {
        items: list?.items,
        rendered: Array.from(
          list?.shadowRoot?.querySelectorAll('.item') ?? [],
        ).map((item: any) => item.textContent),
        shadowsAccessor: Object.hasOwn(list, 'items'),
      };
    });

    expect(result.items).toEqual([
      { name: 'Alpha' },
      { name: 'Beta' },
      { name: 'Gamma' },
    ]);
    expect(result.rendered).toEqual(['Alpha', 'Beta', 'Gamma']);
    expect(result.shadowsAccessor).toBe(false);
  });

  test('queues state for a defined but detached unupgraded child', async ({
    page,
  }) => {
    await page.goto(
      '/complex-prop/fixture.html?definitionOrder=detached-defined-child',
    );
    await page.waitForFunction(() => {
      const host = document.querySelector('#host') as any;
      const list = host?.shadowRoot?.querySelector('test-item-list') as any;
      return host?.$ready === true && list?.$ready === true;
    });

    const result = await page.evaluate(() => {
      const host = document.querySelector('#host') as any;
      const list = host?.shadowRoot?.querySelector('test-item-list') as any;
      return {
        items: list?.items,
        rendered: Array.from(
          list?.shadowRoot?.querySelectorAll('.item') ?? [],
        ).map((item: any) => item.textContent),
        shadowsAccessor: Object.hasOwn(list, 'items'),
      };
    });

    expect(result.items).toEqual([
      { name: 'Alpha' },
      { name: 'Beta' },
      { name: 'Gamma' },
    ]);
    expect(result.rendered).toEqual(['Alpha', 'Beta', 'Gamma']);
    expect(result.shadowsAccessor).toBe(false);
  });
});

test.describe('complex-prop: SSR conditional blocks trust server-rendered content', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/complex-prop/fixture.html');
    await page.waitForFunction(() => {
      const el = document.querySelector('#host') as any;
      return el && el.$ready === true;
    });
  });

  test('SSR conditional driven by :data is hydrated without duplicates', async ({ page }) => {
    // The child's <if condition="data.showHeader"> was rendered by SSR
    // (condData.showHeader=true in state.json). During hydration the child
    // may not yet have its :data set (parent sets it after child hydrates).
    // The framework must trust the SSR content and not create a duplicate.
    const count = await page.evaluate(() => {
      const host = document.querySelector('#host') as any;
      const child = host?.shadowRoot?.querySelector('test-cond-child');
      return child?.shadowRoot?.querySelectorAll('.cond-header')?.length;
    });

    expect(count).toBe(1);
  });

  test('SSR conditional content has correct text from :data', async ({ page }) => {
    const text = await page.evaluate(() => {
      const host = document.querySelector('#host') as any;
      const child = host?.shadowRoot?.querySelector('test-cond-child');
      return child?.shadowRoot?.querySelector('.cond-header')?.textContent;
    });

    expect(text).toBe('Hello');
  });

  test('toggling :data.showHeader to false removes the conditional block', async ({ page }) => {
    await page.evaluate(() => {
      const host = document.querySelector('#host') as any;
      host.hideCondHeader();
    });

    await page.waitForFunction(() => {
      const host = document.querySelector('#host') as any;
      const child = host?.shadowRoot?.querySelector('test-cond-child');
      return child?.shadowRoot?.querySelectorAll('.cond-header')?.length === 0;
    }, null, { timeout: 2000 });

    const count = await page.evaluate(() => {
      const host = document.querySelector('#host') as any;
      const child = host?.shadowRoot?.querySelector('test-cond-child');
      return child?.shadowRoot?.querySelectorAll('.cond-header')?.length;
    });

    expect(count).toBe(0);
  });
});
