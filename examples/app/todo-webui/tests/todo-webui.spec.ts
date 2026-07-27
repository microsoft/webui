// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { test, expect } from '@playwright/test';

// ─── Helper ────────────────────────────────────────────────────
// Playwright locators already pierce shadow DOM by default, so
// `page.locator('todo-item .title')` works in both light and shadow mode.
// For page.evaluate calls we use this helper to get the render root.

/** Return the render root of a custom element (shadowRoot or the element). */
function rootOf(selector: string): string {
  return `(document.querySelector('${selector}')?.shadowRoot ?? document.querySelector('${selector}'))`;
}

// ═══════════════════════════════════════════════════════════════
//  SSR rendering
// ═══════════════════════════════════════════════════════════════

test.describe('SSR rendering', () => {
  test('renders heading and initial state', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('h1')).toContainText('Todo List');
    await expect(page.getByText('Buy groceries')).toBeVisible();
    await expect(page.getByText('Write documentation')).toBeVisible();
    await expect(page.getByText('Ship feature')).toBeVisible();
    await expect(page.getByText('2 items remaining')).toBeVisible();
  });

  test('renders completed item with state attribute', async ({ page }) => {
    await page.goto('/');
    const doneItem = page.locator('todo-item[state="done"]');
    await expect(doneItem).toBeVisible();
    await expect(doneItem).toContainText('Buy groceries');
  });

  test('compiled templates registered in global registry', async ({ page }) => {
    await page.goto('/');
    const templateNames = await page.evaluate(
      () => Object.keys(window.__webui?.templates ?? {}),
    );
    expect(templateNames).toContain('todo-app');
    expect(templateNames).toContain('todo-item');
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
      };
    });

    expect(meta.h).toContain('w-ref');
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

  test('no console errors on page load', async ({ page }) => {
    const errors: string[] = [];
    page.on('pageerror', (err) => errors.push(err.message));
    page.on('console', (msg) => {
      if (msg.type() === 'error') errors.push(msg.text());
    });
    await page.goto('/');
    await page.waitForTimeout(500);
    const real = errors.filter(e => !e.includes('favicon'));
    expect(real).toEqual([]);
  });

  test('fires webui:hydration-complete event', async ({ page }) => {
    // Install listener before page loads to avoid race condition
    await page.goto('/', {
      waitUntil: 'load',
    });

    // By the time 'load' fires, module scripts have executed and
    // hydration is complete.  Check the global performance measure.
    const totalDuration = await page.evaluate(() => {
      const total = performance.getEntriesByName('webui:hydrate:total', 'measure');
      return total[0]?.duration ?? -1;
    });

    expect(totalDuration).toBeGreaterThanOrEqual(0);
  });

  test('no hydration comment markers in output', async ({ page }) => {
    await page.goto('/');
    const result = await page.evaluate(() => {
      const root = document.querySelector('todo-app')?.shadowRoot
        ?? document.querySelector('todo-app');
      if (!root) return { error: 'no root' };

      const walker = document.createTreeWalker(root, NodeFilter.SHOW_COMMENT);
      const markers: string[] = [];
      let c: Comment | null;
      while ((c = walker.nextNode() as Comment | null)) {
        if (c.data.startsWith('w-b:') || c.data.startsWith('w-r:')) {
          markers.push(c.data);
        }
      }

      const dataAttrs: string[] = [];
      for (const el of root.querySelectorAll('*')) {
        for (const attr of el.attributes) {
          if (attr.name.startsWith('data-w-') || attr.name === 'data-ev') {
            dataAttrs.push(`${el.tagName}:${attr.name}`);
          }
        }
      }

      return { markers, dataAttrs };
    });

    expect(result.markers ?? []).toEqual([]);
    expect(result.dataAttrs ?? []).toEqual([]);
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

  test('toggle a todo item done', async ({ page }) => {
    await page.goto('/');
    await page.waitForFunction(
      () => (document.querySelector('todo-app') as any)?.$ready === true,
    );

    const item = page.locator('todo-item[title="Write documentation"]');
    await expect(item).toHaveAttribute('state', 'pending');

    await item.locator('button.toggle').click();

    await expect(item).toHaveAttribute('state', 'done');
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

  test('delete a todo item', async ({ page }) => {
    await page.goto('/');
    await page.waitForFunction(
      () => (document.querySelector('todo-app') as any)?.$ready === true,
    );

    const shipItem = page.locator('todo-item[title="Ship feature"]');
    await expect(shipItem).toBeVisible();
    await shipItem.hover();
    await shipItem.locator('button.delete').click();
    await expect(shipItem).not.toBeAttached();
  });

  test('new item renders title text', async ({ page }) => {
    await page.goto('/');
    await page.waitForFunction(
      () => (document.querySelector('todo-app') as any)?.$ready === true,
    );

    await page.evaluate(() => {
      const app = document.querySelector('todo-app') as any;
      app.addInput.value = 'Visible title test';
      app.addInput.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await page.locator('button.add-button').click();

    const titleText = await page.locator(
      'todo-item[title="Visible title test"] .title',
    ).textContent();
    expect(titleText).toBe('Visible title test');
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