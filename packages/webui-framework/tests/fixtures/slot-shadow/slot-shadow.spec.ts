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
      };
    });

    expect(result.hasShadow).toBe(true);
    expect(result.shadowHasButton).toBe(true);
    // Light DOM children (the icon span and label span) stay in place
    expect(result.lightDomChildren).toBeGreaterThanOrEqual(2);
    expect(result.slotText).toContain('Reply');
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
      };
    });

    expect(result.hasShadow).toBe(true);
    expect(result.shadowHasButton).toBe(true);
    expect(result.slotText).toContain('Reply');
  });
});
