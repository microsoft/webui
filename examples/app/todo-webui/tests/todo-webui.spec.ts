// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { test, expect } from '@playwright/test';

// ─── Helper ────────────────────────────────────────────────────
// Playwright locators already pierce shadow DOM by default, so
// `page.locator('todo-item .title')` works for both Light and Shadow components.
// For page.evaluate calls we use this helper to get the render root.

/** Return the render root of a custom element (shadowRoot or the element). */
function rootOf(selector: string): string {
  return `(document.querySelector('${selector}')?.shadowRoot ?? document.querySelector('${selector}'))`;
}

// ═══════════════════════════════════════════════════════════════
//  SSR rendering
// ═══════════════════════════════════════════════════════════════

test.describe('SSR rendering', () => {
  test('renders completed item with state attribute', async ({ page }) => {
    await page.goto('/');
    const doneItem = page.locator('todo-item[state="done"]');
    await expect(doneItem).toBeVisible();
    await expect(doneItem).toContainText('Buy groceries');
  });

  test('compiled template is a metadata object', async ({ page }) => {
    await page.goto('/');
    const meta = await page.evaluate(() => {
      const template = window.__webui?.templates?.['todo-app'];
      const textPaths = Array.isArray(template?.tx)
        ? template.tx
          .flatMap(([, parts]) => parts)
          .filter((part: unknown) => Array.isArray(part))
          .map(([path]: [string]) => path)
        : [];

      return {
        h: template?.h ?? '',
        textPaths,
        repeat: template?.r?.[0] ?? null,
        hasRepeatSlot: Array.isArray(template?.r?.[0]) && template.r[0].length >= 4,
        eventCount: template?.eg?.reduce((count, [, bindings]) => count + bindings.length, 0) ?? 0,
        hasEvents: Array.isArray(template?.eg),
        refs: template?.rf?.map(([name]) => name) ?? [],
      };
    });

    expect(meta.h).not.toContain('w-ref');
    expect(meta.refs).toContain('addInput');
    expect(meta.h).not.toContain('data-w-');
    expect(meta.h).not.toContain('data-ev');
    expect(meta.h).not.toContain('{{');
    expect(meta.h).not.toContain('<for');
    expect(meta.h).not.toContain('<if');

    expect(meta.textPaths).toContain('title');
    expect(meta.textPaths).toContain('remainingCount');

    expect(meta.repeat).not.toBeNull();
    expect(meta.repeat[0]).toBe('items');
    expect(meta.repeat[1]).toBe('item');
    expect(meta.hasRepeatSlot).toBe(true);

    expect(meta.eventCount).toBeGreaterThan(0);
    expect(meta.hasEvents).toBe(true);
  });

});

// ═══════════════════════════════════════════════════════════════
//  Interactivity
// ═══════════════════════════════════════════════════════════════

test.describe('interactivity', () => {
  test('add a new todo item via Add button', async ({ page }) => {
    await page.goto('/');
    await page.waitForFunction(
      () => (document.querySelector('todo-app') as any)?.$ready === true,
    );

    await page.evaluate(() => {
      const app = document.querySelector('todo-app') as any;
      if (app.addInput !== app.shadowRoot.querySelector('input.add-input')) {
        throw new Error('The compiled addInput ref must target the hydrated input.');
      }
      app.addInput.value = 'Write Playwright tests';
      app.addInput.dispatchEvent(new Event('input', { bubbles: true }));
    });
    // Playwright locator pierces shadow DOM automatically
    await page.locator('button.add-button').click();

    const newItem = page.locator('todo-item[title="Write Playwright tests"]');
    await expect(newItem).toBeAttached();
    await expect(newItem).toHaveAttribute('state', 'pending');

    // New pending item should NOT have a checkmark
    await expect(newItem.locator('.check')).not.toBeAttached();
  });

  test('add a todo via Enter key', async ({ page }) => {
    await page.goto('/');
    await page.waitForFunction(
      () => (document.querySelector('todo-app') as any)?.$ready === true,
    );

    await page.evaluate(() => {
      const app = document.querySelector('todo-app') as any;
      app.addInput.value = 'Press Enter todo';
      app.addInput.dispatchEvent(
        new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }),
      );
    });

    await expect(page.locator('todo-item[title="Press Enter todo"]')).toBeAttached();
  });

  test('if-block checkmark appears when toggled to done', async ({ page }) => {
    await page.goto('/');
    await page.waitForFunction(
      () => (document.querySelector('todo-app') as any)?.$ready === true,
    );

    const item = page.locator('todo-item[title="Write documentation"]');
    await expect(item.locator('.check')).not.toBeAttached();

    await item.locator('button.toggle').click();

    await expect(item.locator('.check')).toBeAttached();
  });

  test('if-block checkmark disappears when toggled back to pending', async ({ page }) => {
    await page.goto('/');
    await page.waitForFunction(
      () => (document.querySelector('todo-app') as any)?.$ready === true,
    );

    const item = page.locator('todo-item[title="Buy groceries"]');
    await expect(item.locator('.check')).toBeAttached();

    await item.locator('button.toggle').click();

    await expect(item.locator('.check')).not.toBeAttached();
  });

  test('deleting all items leaves empty list', async ({ page }) => {
    await page.goto('/');
    await page.waitForFunction(
      () => (document.querySelector('todo-app') as any)?.$ready === true,
    );

    for (const title of ['Buy groceries', 'Write documentation', 'Ship feature']) {
      const item = page.locator(`todo-item[title="${title}"]`);
      await item.hover();
      await item.locator('button.delete').click();
      await expect(item).not.toBeAttached();
    }

    await expect(page.locator('todo-item')).toHaveCount(0);
    await expect(page.getByText('0 items remaining')).toBeVisible();
  });

  test('remaining count updates reactively', async ({ page }) => {
    await page.goto('/');
    await page.waitForFunction(
      () => (document.querySelector('todo-app') as any)?.$ready === true,
    );

    await expect(page.getByText('2 items remaining')).toBeVisible();

    await page.evaluate(() => {
      const app = document.querySelector('todo-app') as any;
      app.addInput.value = 'Count test';
      app.addInput.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await page.locator('button.add-button').click();

    await expect(page.getByText('3 items remaining')).toBeVisible();
  });

  test('dynamically-created item shows checkmark when toggled', async ({ page }) => {
    await page.goto('/');
    await page.waitForFunction(
      () => (document.querySelector('todo-app') as any)?.$ready === true,
    );

    // Add item
    await page.evaluate(() => {
      const app = document.querySelector('todo-app') as any;
      app.addInput.value = 'Dynamic toggle';
      app.addInput.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await page.locator('button.add-button').click();

    const item = page.locator('todo-item[title="Dynamic toggle"]');
    await expect(item).toBeAttached();

    // Toggle to done
    await item.locator('button.toggle').click();
    await expect(item.locator('.check')).toBeAttached();

    // Toggle back to pending
    await item.locator('button.toggle').click();
    await expect(item.locator('.check')).not.toBeAttached();
  });
});