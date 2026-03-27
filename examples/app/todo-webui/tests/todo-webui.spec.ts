// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { test, expect } from '@playwright/test';

test.describe('SSR rendering', () => {
  test('renders heading and initial state', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('h1')).toContainText('Todo List');
    await expect(page.getByText('Buy groceries')).toBeVisible();
    await expect(page.getByText('Write documentation')).toBeVisible();
    await expect(page.getByText('Ship feature')).toBeVisible();
    await expect(page.getByText('2 items remaining')).toBeVisible();
  });

  test('renders completed item with strikethrough', async ({ page }) => {
    await page.goto('/');
    // "Buy groceries" has state="done" — its todo-item host should have that attribute
    const doneItem = page.locator('todo-item[state="done"]');
    await expect(doneItem).toBeVisible();
    await expect(doneItem).toContainText('Buy groceries');
  });

  test('compiled templates registered in global registry', async ({ page }) => {
    await page.goto('/');
    const templateNames = await page.evaluate(
      () => Object.keys(window.__webui_templates ?? {}),
    );
    expect(templateNames).toContain('todo-app');
    expect(templateNames).toContain('todo-item');
  });

  test('compiled template is a metadata object', async ({ page }) => {
    await page.goto('/');
    const meta = await page.evaluate(() => {
      const template = window.__webui_templates?.['todo-app'];
      const textPaths = Array.isArray(template?.tx)
        ? template.tx
          .flatMap(([, parts]) => parts)
          .filter((part) => Array.isArray(part))
          .map(([path]) => path)
        : [];

      return {
        h: template?.h ?? '',
        textPaths,
        repeat: template?.r?.[0] ?? null,
        hasRepeatSlots: Array.isArray(template?.rl),
        eventCount: template?.e?.length ?? 0,
        hasEventTargets: Array.isArray(template?.el),
      };
    });

    // h: marker-free static HTML with only static attributes left in place
    expect(meta.h).toContain('w-ref');
    expect(meta.h).not.toContain('<!--t:');
    expect(meta.h).not.toContain('<!--c:');
    expect(meta.h).not.toContain('<!--r:');
    expect(meta.h).not.toContain('data-w-');
    expect(meta.h).not.toContain('data-ev');
    expect(meta.h).not.toContain('{{');
    expect(meta.h).not.toContain('<for');
    expect(meta.h).not.toContain('<if');
    expect(meta.h).not.toContain('shadowrootmode');

    // tx: locator-driven text runs
    expect(meta.textPaths).toContain('title');
    expect(meta.textPaths).toContain('remainingCount');

    // r/rl: repeat bindings plus anchor slots
    expect(meta.repeat).not.toBeNull();
    expect(meta.repeat[0]).toBe('items'); // collection
    expect(meta.repeat[1]).toBe('item');  // item var
    expect(meta.hasRepeatSlots).toBe(true);

    // e/el: events plus target locators
    expect(meta.eventCount).toBeGreaterThan(0);
    expect(meta.hasEventTargets).toBe(true);
  });

  test('no console errors on page load', async ({ page }) => {
    const errors: string[] = [];
    page.on('pageerror', (err) => errors.push(err.message));
    page.on('console', (msg) => {
      if (msg.type() === 'error') errors.push(msg.text());
    });
    await page.goto('/');
    await page.waitForTimeout(500);
    expect(errors).toEqual([]);
  });

  test('fires webui:hydration-complete event with performance marks', async ({ page }) => {
    await page.goto('/');
    // Wait for the hydration-complete event to have fired
    const result = await page.evaluate(() =>
      new Promise<Record<string, unknown>>((resolve) => {
        // If already complete, read immediately
        const total = performance.getEntriesByName('webui:hydrate:total', 'measure');
        if (total.length > 0) {
          const perComponent = performance.getEntriesByType('measure')
            .filter(e => e.name.startsWith('webui:hydrate:') && e.name !== 'webui:hydrate:total')
            .map(e => e.name);
          return resolve({
            totalDuration: total[0].duration,
            components: perComponent,
          });
        }
        // Otherwise wait
        window.addEventListener('webui:hydration-complete', () => {
          const t = performance.getEntriesByName('webui:hydrate:total', 'measure');
          const perComp = performance.getEntriesByType('measure')
            .filter(e => e.name.startsWith('webui:hydrate:') && e.name !== 'webui:hydrate:total')
            .map(e => e.name);
          resolve({ totalDuration: t[0]?.duration, components: perComp });
        });
      }),
    );

    expect(result.totalDuration).toBeGreaterThanOrEqual(0);
    const components = result.components as string[];
    // Should have hydrated todo-app and at least 3 todo-items
    expect(components.some(c => c.includes('todo-app'))).toBe(true);
    expect(components.some(c => c.includes('todo-item'))).toBe(true);
    expect(components.length).toBeGreaterThanOrEqual(4);
  });

  test('one-shot hydration markers removed while repeat anchors stay available', async ({ page }) => {
    await page.goto('/');
    const result = await page.evaluate(() => {
      const app = document.querySelector('todo-app');
      const sr = app?.shadowRoot;
      if (!sr) return { error: 'no shadow root' };

      // Check for any remaining hydration comments
      const walker = document.createTreeWalker(sr, NodeFilter.SHOW_COMMENT);
      const bindingComments: string[] = [];
      const repeatComments: string[] = [];
      let c: Comment | null;
      while ((c = walker.nextNode() as Comment | null)) {
        if (!(c.data.startsWith('w-b:') || c.data.startsWith('w-r:'))) {
          continue;
        }

        if (c.data.includes(':for-') || c.data.startsWith('w-r:')) {
          repeatComments.push(c.data);
        } else {
          bindingComments.push(c.data);
        }
      }

      // Check for any data-w-* attributes
      const dataWAttrs: string[] = [];
      for (const el of sr.querySelectorAll('*')) {
        for (const attr of el.attributes) {
          if (attr.name.startsWith('data-w-')) {
            dataWAttrs.push(`${el.tagName}:${attr.name}`);
          }
        }
      }

      return { bindingComments, repeatComments, dataWAttrs };
    });

    expect(result.bindingComments).toEqual([]);
    expect(result.repeatComments.length).toBeGreaterThan(0);
    expect(result.dataWAttrs).toEqual([]);
  });

  test('reactive text bindings still work after one-shot markers are removed', async ({ page }) => {
    await page.goto('/');

    // Verify text/conditional hydration markers are gone even though repeat
    // anchors can remain for reconciliation.
    const markersGone = await page.evaluate(() => {
      const sr = document.querySelector('todo-app')?.shadowRoot;
      const walker = document.createTreeWalker(sr!, NodeFilter.SHOW_COMMENT);
      let c;
      while ((c = walker.nextNode())) {
        const data = (c as Comment).data;
        if ((data.startsWith('w-b:') || data.startsWith('w-r:')) && !data.includes(':for-')) {
          return false;
        }
      }
      return true;
    });
    expect(markersGone).toBe(true);

    // Now trigger a reactive update — add a todo item
    await page.evaluate(() => {
      const app = document.querySelector('todo-app')!;
      (app as any).addInput.value = 'After marker removal';
      (app as any).addInput.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await page.evaluate(() => {
      document.querySelector('todo-app')!.shadowRoot!
        .querySelector<HTMLButtonElement>('.add-button')!.click();
    });

    // The new item should appear (repeat reconciliation works)
    await expect(page.locator('todo-item[title="After marker removal"]')).toBeAttached();

    // Remaining count should have updated reactively (text binding works)
    const remaining = await page.evaluate(() => {
      const app = document.querySelector('todo-app');
      return (app as any).remainingCount;
    });
    expect(remaining).toBe(3); // 2 original pending + 1 new
  });
});

test.describe('interactivity', () => {
  test('add a new todo item', async ({ page }) => {
    await page.goto('/');
    await page.waitForFunction(
      () => document.querySelector('todo-app')?.addInput instanceof HTMLInputElement,
    );

    // Fill and click inside shadow DOM
    await page.evaluate(() => {
      const app = document.querySelector('todo-app')!;
      app.addInput.value = 'Write Playwright tests';
      app.addInput.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await page.evaluate(() => {
      document.querySelector('todo-app')!.shadowRoot!.querySelector<HTMLButtonElement>('.add-button')!.click();
    });

    // The new item appears with pending state and no checkmark
    const newItem = page.locator('todo-item[title="Write Playwright tests"]');
    await expect(newItem).toBeAttached();
    await expect(newItem).toHaveAttribute('state', 'pending');

    const hasCheck = await page.evaluate(() => {
      const el = document.querySelector('todo-app')!.shadowRoot!
        .querySelector('todo-item[title="Write Playwright tests"]');
      return !!el?.shadowRoot?.querySelector('.check');
    });
    expect(hasCheck).toBe(false);
  });

  test('new dynamically-created item shows checkmark when toggled to done', async ({ page }) => {
    await page.goto('/');
    await page.waitForFunction(
      () => document.querySelector('todo-app')?.addInput instanceof HTMLInputElement,
    );

    // Add a new item
    await page.evaluate(() => {
      const app = document.querySelector('todo-app')!;
      (app as any).addInput.value = 'Dynamic toggle test';
      (app as any).addInput.dispatchEvent(new Event('input', { bubbles: true }));
      app.shadowRoot!.querySelector<HTMLButtonElement>('.add-button')!.click();
    });
    await expect(page.locator('todo-item[title="Dynamic toggle test"]')).toBeAttached();

    // Toggle it to done via custom event (simulating click → $emit flow)
    await page.evaluate(() => {
      const app = document.querySelector('todo-app')!;
      const item = app.shadowRoot!.querySelector('todo-item[title="Dynamic toggle test"]')!;
      item.dispatchEvent(new CustomEvent('toggle-item', {
        bubbles: true, composed: true,
        detail: { id: item.getAttribute('id') },
      }));
    });

    // Checkmark should appear
    const hasCheck = await page.evaluate(() => {
      const item = document.querySelector('todo-app')!.shadowRoot!
        .querySelector('todo-item[title="Dynamic toggle test"]');
      return !!item?.shadowRoot?.querySelector('.check');
    });
    expect(hasCheck).toBe(true);
  });

  test('add a todo via Enter key', async ({ page }) => {
    await page.goto('/');
    await page.waitForFunction(
      () => document.querySelector('todo-app')?.addInput instanceof HTMLInputElement,
    );

    await page.evaluate(() => {
      const app = document.querySelector('todo-app')!;
      app.addInput.value = 'Press Enter todo';
      app.addInput.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    });

    await expect(page.locator('todo-item[title="Press Enter todo"]')).toBeAttached();
  });

  test('toggle a todo item done', async ({ page }) => {
    await page.goto('/');
    await page.waitForFunction(() => customElements.get('todo-item') !== undefined);

    const writeDocItem = page.locator('todo-item[title="Write documentation"]');
    await expect(writeDocItem).toHaveAttribute('state', 'pending');

    const toggleBtn = writeDocItem.locator('button.toggle');
    await toggleBtn.click();

    await expect(writeDocItem).toHaveAttribute('state', 'done');
  });

  test('if-block checkmark appears when toggled to done', async ({ page }) => {
    await page.goto('/');
    await page.waitForFunction(() => customElements.get('todo-item') !== undefined);

    // "Write documentation" starts pending — no checkmark
    const hasCheckBefore = await page.evaluate(() => {
      const item = document.querySelector('todo-app')!.shadowRoot!
        .querySelector('todo-item[title="Write documentation"]');
      return !!item?.shadowRoot?.querySelector('.check');
    });
    expect(hasCheckBefore).toBe(false);

    // Toggle to done — checkmark should appear reactively
    await page.evaluate(() => {
      const item = document.querySelector('todo-app')!.shadowRoot!
        .querySelector('todo-item[title="Write documentation"]');
      item?.shadowRoot?.querySelector<HTMLButtonElement>('.toggle')?.click();
    });

    const hasCheckAfter = await page.evaluate(() => {
      const item = document.querySelector('todo-app')!.shadowRoot!
        .querySelector('todo-item[title="Write documentation"]');
      return !!item?.shadowRoot?.querySelector('.check');
    });
    expect(hasCheckAfter).toBe(true);
  });

  test('if-block checkmark disappears when toggled back to pending', async ({ page }) => {
    await page.goto('/');
    await page.waitForFunction(() => customElements.get('todo-item') !== undefined);

    // "Buy groceries" starts done — has checkmark
    const hasCheckBefore = await page.evaluate(() => {
      const item = document.querySelector('todo-app')!.shadowRoot!
        .querySelector('todo-item[title="Buy groceries"]');
      return !!item?.shadowRoot?.querySelector('.check');
    });
    expect(hasCheckBefore).toBe(true);

    // Toggle to pending — checkmark should disappear reactively
    await page.evaluate(() => {
      const item = document.querySelector('todo-app')!.shadowRoot!
        .querySelector('todo-item[title="Buy groceries"]');
      item?.shadowRoot?.querySelector<HTMLButtonElement>('.toggle')?.click();
    });

    const hasCheckAfter = await page.evaluate(() => {
      const item = document.querySelector('todo-app')!.shadowRoot!
        .querySelector('todo-item[title="Buy groceries"]');
      return !!item?.shadowRoot?.querySelector('.check');
    });
    expect(hasCheckAfter).toBe(false);
  });

  test('delete a todo item', async ({ page }) => {
    await page.goto('/');
    await page.waitForFunction(() => customElements.get('todo-item') !== undefined);

    await expect(page.getByText('Ship feature')).toBeVisible();

    const shipItem = page.locator('todo-item[title="Ship feature"]');
    await shipItem.hover();
    const deleteBtn = shipItem.locator('button.delete');
    await deleteBtn.click();

    await expect(page.getByText('Ship feature')).not.toBeVisible();
  });

  test('new dynamically-created item renders title text inside shadow DOM', async ({ page }) => {
    await page.goto('/');
    await page.waitForFunction(
      () => document.querySelector('todo-app')?.addInput instanceof HTMLInputElement,
    );

    // Add a new item
    await page.evaluate(() => {
      const app = document.querySelector('todo-app')!;
      (app as any).addInput.value = 'Visible title test';
      (app as any).addInput.dispatchEvent(new Event('input', { bubbles: true }));
      app.shadowRoot!.querySelector<HTMLButtonElement>('.add-button')!.click();
    });

    // Verify the title text is actually rendered inside the shadow DOM
    // (regression test: previously blank until a toggle/re-render)
    const titleText = await page.evaluate(() => {
      const item = document.querySelector('todo-app')!.shadowRoot!
        .querySelector('todo-item[title="Visible title test"]');
      return item?.shadowRoot?.querySelector('.title')?.textContent;
    });
    expect(titleText).toBe('Visible title test');
  });

  test('deleting all items leaves empty list', async ({ page }) => {
    await page.goto('/');
    await page.waitForFunction(() => customElements.get('todo-item') !== undefined);

    // Delete all 3 items one by one
    for (const title of ['Buy groceries', 'Write documentation', 'Ship feature']) {
      const item = page.locator(`todo-item[title="${title}"]`);
      await item.hover();
      await item.locator('button.delete').click();
      await expect(item).not.toBeAttached();
    }

    // No todo-items should remain
    const count = await page.evaluate(() => {
      return document.querySelector('todo-app')!.shadowRoot!
        .querySelectorAll('todo-item').length;
    });
    expect(count).toBe(0);

    // Remaining count should be 0
    await expect(page.getByText('0 items remaining')).toBeVisible();
  });
});
