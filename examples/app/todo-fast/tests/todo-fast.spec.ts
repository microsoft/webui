// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, type Page, test } from '@playwright/test';

async function gotoAndWaitForFastHydration(page: Page): Promise<void> {
  await page.goto('/');
  await page.waitForFunction(() => {
    const app = document.querySelector('todo-app');
    return Boolean(
      customElements.get('todo-app')
      && customElements.get('todo-item')
      && app?.shadowRoot?.querySelector('input.add-input'),
    );
  });
}

async function readInitialHtml(page: Page): Promise<string> {
  const response = await page.goto('/');
  expect(response).not.toBeNull();
  return response?.text() ?? '';
}

function fastTemplate(html: string, name: string): string {
  const start = html.indexOf(`<f-template name="${name}">`);
  expect(start, `expected ${name} f-template`).toBeGreaterThanOrEqual(0);
  const end = html.indexOf('</f-template>', start);
  expect(end, `expected ${name} closing f-template`).toBeGreaterThan(start);
  return html.slice(start, end);
}

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

  test('emits FAST templates with root runtime attrs without CSS modules', async ({ page }) => {
    const html = await readInitialHtml(page);
    const appTemplate = fastTemplate(html, 'todo-app');
    const itemTemplate = fastTemplate(html, 'todo-item');

    expect(html).not.toContain('shadowrootadoptedstylesheets');
    expect(appTemplate).toContain('@toggle-item="{onToggleItem($e)}"');
    expect(appTemplate).toContain('@delete-item="{onDeleteItem($e)}"');
    expect(itemTemplate).toContain('@click="{onClick($e)}"');
  });

  test('no console errors on page load', async ({ page }) => {
    const errors: string[] = [];
    page.on('pageerror', err => errors.push(err.message));
    page.on('console', msg => {
      if (msg.type() === 'error') errors.push(msg.text());
    });
    await page.goto('/');
    await page.waitForTimeout(500);
    const real = errors.filter(e => !e.includes('favicon'));
    expect(real).toEqual([]);
  });

  test('fires FAST hydration completion measure', async ({ page }) => {
    await gotoAndWaitForFastHydration(page);
    await page.waitForFunction(() =>
      performance.getEntriesByName('todo-hydration-completed', 'measure').length > 0
    );

    const totalDuration = await page.evaluate(() => {
      const total = performance.getEntriesByName('todo-hydration-completed', 'measure');
      return total[0]?.duration ?? -1;
    });

    expect(totalDuration).toBeGreaterThanOrEqual(0);
  });
});

test.describe('interactivity', () => {
  test('add a new todo item via Add button', async ({ page }) => {
    await gotoAndWaitForFastHydration(page);

    await page.locator('input.add-input').fill('Write Playwright tests');
    await page.locator('button.add-button').click();

    const newItem = page.locator('todo-item[title="Write Playwright tests"]');
    await expect(newItem).toBeAttached();
    await expect(newItem).toHaveAttribute('state', 'pending');
    await expect(newItem.locator('.check')).not.toBeAttached();
  });

  test('add a todo via Enter key', async ({ page }) => {
    await gotoAndWaitForFastHydration(page);

    await page.locator('input.add-input').fill('Press Enter todo');
    await page.locator('input.add-input').press('Enter');

    await expect(page.locator('todo-item[title="Press Enter todo"]')).toBeAttached();
  });

  test('toggle a todo item done', async ({ page }) => {
    await gotoAndWaitForFastHydration(page);

    const item = page.locator('todo-item[title="Write documentation"]');
    await expect(item).toHaveAttribute('state', 'pending');

    await item.locator('button.toggle').click();

    await expect(item).toHaveAttribute('state', 'done');
  });

  test('if-block checkmark appears when toggled to done', async ({ page }) => {
    await gotoAndWaitForFastHydration(page);

    const item = page.locator('todo-item[title="Write documentation"]');
    await expect(item.locator('.check')).not.toBeAttached();

    await item.locator('button.toggle').click();

    await expect(item.locator('.check')).toBeAttached();
  });

  test('if-block checkmark disappears when toggled back to pending', async ({ page }) => {
    await gotoAndWaitForFastHydration(page);

    const item = page.locator('todo-item[title="Buy groceries"]');
    await expect(item.locator('.check')).toBeAttached();

    await item.locator('button.toggle').click();

    await expect(item.locator('.check')).not.toBeAttached();
  });

  test('delete a todo item', async ({ page }) => {
    await gotoAndWaitForFastHydration(page);

    const shipItem = page.locator('todo-item[title="Ship feature"]');
    await expect(shipItem).toBeVisible();
    await shipItem.hover();
    await shipItem.locator('button.delete').click();
    await expect(shipItem).not.toBeAttached();
  });

  test('new item renders title text', async ({ page }) => {
    await gotoAndWaitForFastHydration(page);

    await page.locator('input.add-input').fill('Visible title test');
    await page.locator('button.add-button').click();

    const titleText = await page.locator(
      'todo-item[title="Visible title test"] .title',
    ).textContent();
    expect(titleText).toBe('Visible title test');
  });

  test('deleting all items leaves empty list', async ({ page }) => {
    await gotoAndWaitForFastHydration(page);

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
    await gotoAndWaitForFastHydration(page);

    await expect(page.getByText('2 items remaining')).toBeVisible();

    await page.locator('input.add-input').fill('Count test');
    await page.locator('button.add-button').click();

    await expect(page.getByText('3 items remaining')).toBeVisible();
  });

  test('dynamically-created item shows checkmark when toggled', async ({ page }) => {
    await gotoAndWaitForFastHydration(page);

    await page.locator('input.add-input').fill('Dynamic toggle');
    await page.locator('button.add-button').click();

    const item = page.locator('todo-item[title="Dynamic toggle"]');
    await expect(item).toBeAttached();

    await item.locator('button.toggle').click();
    await expect(item.locator('.check')).toBeAttached();

    await item.locator('button.toggle').click();
    await expect(item.locator('.check')).not.toBeAttached();
  });
});
