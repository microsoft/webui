// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Regression test: shadow-DOM components with pre-existing slot content.
 *
 * When a shadow-DOM component (meta.sd = 1) is created with child nodes
 * already present — as happens during SPA partial rendering — the framework
 * must still create a shadow root.  A prior bug caused $mount to
 * misidentify slot children as SSR light-DOM content, skipping shadow root
 * creation entirely.
 */

import { expect, test } from '@playwright/test';

interface ConditionalSlotOwnership {
  rows: number;
  menuItems: number;
  previewItems: number;
  logItems: number;
  restartItems: number;
  stopItems: number;
  misplacedInTable: number;
  allOwnedByMenu: boolean;
  allAssignedToMenuSlot: boolean;
  allMenusAssignedToOuterSlot: boolean;
}

function readConditionalSlotOwnership(): ConditionalSlotOwnership {
  const parent = document.querySelector('#parent');
  const root = parent?.shadowRoot;
  const menus = Array.from(root?.querySelectorAll('mai-menu-list') ?? []);
  const items = Array.from(root?.querySelectorAll('mai-menu-item') ?? []);

  return {
    rows: root?.querySelectorAll('.resource-row').length ?? 0,
    menuItems: items.length,
    previewItems: root?.querySelectorAll('.preview-item').length ?? 0,
    logItems: root?.querySelectorAll('.logs-item').length ?? 0,
    restartItems: root?.querySelectorAll('.restart-item').length ?? 0,
    stopItems: root?.querySelectorAll('.stop-item').length ?? 0,
    misplacedInTable:
      root?.querySelectorAll('table > mai-menu-item, tbody > mai-menu-item')
        .length ?? 0,
    allOwnedByMenu: items.every(
      item => item.parentElement?.localName === 'mai-menu-list',
    ),
    allAssignedToMenuSlot: items.every(
      item => item.assignedSlot?.localName === 'slot',
    ),
    allMenusAssignedToOuterSlot: menus.every(
      menu => menu.assignedSlot?.localName === 'slot',
    ),
  };
}

const expectedConditionalSlotOwnership: ConditionalSlotOwnership = {
  rows: 2,
  menuItems: 7,
  previewItems: 1,
  logItems: 2,
  restartItems: 2,
  stopItems: 2,
  misplacedInTable: 0,
  allOwnedByMenu: true,
  allAssignedToMenuSlot: true,
  allMenusAssignedToOuterSlot: true,
};

const expectedSSRConditionalSlotOwnership: ConditionalSlotOwnership = {
  ...expectedConditionalSlotOwnership,
  allAssignedToMenuSlot: false,
  allMenusAssignedToOuterSlot: false,
};

const expectedUpdatedConditionalSlotOwnership: ConditionalSlotOwnership = {
  ...expectedConditionalSlotOwnership,
  menuItems: 6,
  previewItems: 1,
  logItems: 1,
};

test.describe('slot-shadow: SPA partial regression', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/slot-shadow/fixture.html');
    await page.waitForSelector('test-slot-btn');
    await page.waitForFunction(() => {
      const el = document.querySelector('#empty-child') as any;
      return el && el.$ready === true;
    });
  });

  test('empty child gets a shadow root (baseline)', async ({ page }) => {
    const hasShadow = await page.evaluate(() => {
      const el = document.querySelector('#empty-child');
      return !!el?.shadowRoot;
    });
    expect(hasShadow).toBe(true);
  });

  test('authored Shadow wrapper creates the parent shadow root', async ({ page }) => {
    const hasShadow = await page.locator('#parent').evaluate(
      element => !!element.shadowRoot,
    );
    expect(hasShadow).toBe(true);
  });

  test('child with pre-existing slot content gets a shadow root', async ({ page }) => {
    // Wait for the preloaded child to be ready
    await page.waitForFunction(() => {
      const el = document.querySelector('#preloaded-child') as any;
      return el && el.$ready === true;
    });

    const result = await page.evaluate(() => {
      const el = document.querySelector('#preloaded-child');
      return {
        hasShadow: !!el?.shadowRoot,
        // The shadow root should contain the <button class="btn"><slot></slot></button>
        shadowHasButton: !!el?.shadowRoot?.querySelector('button.btn'),
        // The slot content should still be in the light DOM
        lightDomChildren: el?.children.length,
        // Slot content should be projected
        slotText: el?.textContent?.trim(),
        projected:
          el?.querySelector('span:not(.appearance)')?.assignedSlot?.localName,
      };
    });

    expect(result.hasShadow).toBe(true);
    expect(result.shadowHasButton).toBe(true);
    // Light DOM children (the icon span and label span) stay in place
    expect(result.lightDomChildren).toBeGreaterThanOrEqual(2);
    expect(result.slotText).toContain('Reply');
    expect(result.projected).toBe('slot');
    await expect(page.locator('#preloaded-child .appearance')).toHaveText('primary');

    await page.locator('#preloaded-child').evaluate((element) => {
      (element as HTMLElement & { appearance: string }).appearance = 'secondary';
    });
    await expect(page.locator('#preloaded-child .appearance')).toHaveText('secondary');
    await expect(page.locator('#preloaded-child')).toContainText('Reply');
  });

  test('keeps conditional custom items owned by their slotted menu across hydration', async ({
    browser,
    page,
  }) => {
    const ssrContext = await browser.newContext({ javaScriptEnabled: false });
    const ssrPage = await ssrContext.newPage();
    await ssrPage.goto('/slot-shadow/fixture.html');

    expect(await ssrPage.evaluate(readConditionalSlotOwnership)).toEqual(
      expectedSSRConditionalSlotOwnership,
    );
    await ssrContext.close();

    await page.waitForFunction(() => {
      const parent = document.querySelector('#parent') as any;
      const menus = parent?.shadowRoot?.querySelectorAll('mai-menu-list');
      const items = parent?.shadowRoot?.querySelectorAll('mai-menu-item');
      return menus?.length === 2
        && items?.length >= 7
        && Array.from(menus).every(menu => (menu as Element).shadowRoot)
        && Array.from(items).every(item => (item as Element).shadowRoot);
    });

    expect(await page.evaluate(readConditionalSlotOwnership)).toEqual(
      expectedConditionalSlotOwnership,
    );
    await expect(page.locator('#parent .preview-item')).toBeVisible();
    await expect(page.locator('#parent .logs-item')).toHaveCount(2);
    await expect(page.locator('#parent .restart-item')).toHaveCount(2);
    await expect(page.locator('#parent .stop-item')).toHaveCount(2);

    await page.locator('#parent').evaluate((element) => {
      (element as any).previews = [
        { preview_url: '', can_view_logs: false },
        { preview_url: '/preview/starting', can_view_logs: true },
      ];
    });

    await expect.poll(
      () => page.evaluate(readConditionalSlotOwnership),
    ).toEqual(expectedUpdatedConditionalSlotOwnership);
  });

  test('dynamically spawned child with slot content gets a shadow root', async ({ page }) => {
    // Trigger the parent to spawn a child with slot content
    await page.evaluate(() => {
      const parent = document.querySelector('#parent') as any;
      parent.spawnSlotChild();
    });

    // Wait for the spawned child to be ready
    await page.waitForFunction(() => {
      const parent = document.querySelector('#parent') as any;
      const root = parent?.shadowRoot;
      if (!root) return false;
      const child = root.querySelector('test-slot-btn') as any;
      return child && child.$ready === true;
    });

    const result = await page.evaluate(() => {
      const parent = document.querySelector('#parent') as any;
      const child = parent?.shadowRoot?.querySelector('test-slot-btn');
      return {
        hasShadow: !!child?.shadowRoot,
        shadowHasButton: !!child?.shadowRoot?.querySelector('button.btn'),
        slotText: child?.textContent?.trim(),
        projected:
          child?.querySelector('span:not(.appearance)')?.assignedSlot?.localName,
      };
    });

    expect(result.hasShadow).toBe(true);
    expect(result.shadowHasButton).toBe(true);
    expect(result.slotText).toContain('Reply');
    expect(result.projected).toBe('slot');
    await expect(
      page.locator('test-slot-parent test-slot-btn .appearance'),
    ).toHaveText('primary');
  });
});
