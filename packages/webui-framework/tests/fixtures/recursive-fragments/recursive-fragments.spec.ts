// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';
import type {
  TestRecursiveLight,
  TestRecursiveTable,
  TestRecursiveTree,
  TestRecursiveUnknown,
} from './element.js';

test.beforeEach(async ({ page }) => {
  await page.goto('/recursive-fragments/fixture.html');
  await page.waitForFunction(() =>
    (document.querySelector('test-recursive-tree') as unknown as { $ready: boolean })?.$ready,
  );
});

test('real SSR hydrates recursive calls without wrappers or duplicated nodes', async ({ page, request }) => {
  const response = await request.get('/recursive-fragments/fixture.html');
  const html = await response.text();
  expect(html).toContain('<!--wf-->');
  expect(html).toContain('<!--/wf-->');
  expect(html).not.toContain('<fragment');
  expect(html).not.toContain('<render');
  const tree = page.locator('test-recursive-tree');
  await expect(tree.locator('h2')).toHaveText('Forest');
  await expect(tree.locator('.name')).toHaveText(['Oak', 'Leaf', 'Bud', 'Pine']);
  await tree.locator('[data-id="leaf"] > .select').click();
  await expect(tree.locator('.selected')).toHaveText('leaf:Leaf');
});

test('loop-member fragment inputs preserve native owner fallback through hydration and updates', async ({ page, request }) => {
  const response = await request.get('/recursive-fragments/fixture.html');
  expect(await response.text()).toContain('GLOBAL|GLOBAL');
  const light = page.locator('test-recursive-light');
  await expect(light).toHaveJSProperty('$ready', true);
  await expect(light.locator('.loop-fallback')).toHaveText('GLOBAL|GLOBAL');
  await light.evaluate((element: TestRecursiveLight) => {
    element.item = { fallback: 'CHANGED' };
  });
  await expect(light.locator('.loop-fallback')).toHaveText('CHANGED|CHANGED');
  await light.evaluate((element: TestRecursiveLight) => {
    element.fallbackItems = [{ fallback: 'LOCAL' }];
  });
  await expect(light.locator('.loop-fallback')).toHaveText('LOCAL|CHANGED');
  await light.evaluate((element: TestRecursiveLight) => {
    element.item = { fallback: 'OWNER' };
  });
  await expect(light.locator('.loop-fallback')).toHaveText('LOCAL|OWNER');
});

test('root and leaf writes update every call and no-op updates keep identity', async ({ page }) => {
  const tree = page.locator('test-recursive-tree');
  const leaf = tree.locator('[data-id="leaf"] > input');
  const original = await leaf.elementHandle();
  await page.evaluate(() => {
    const element = document.querySelector('test-recursive-tree') as TestRecursiveTree;
    element.title = 'Updated forest';
    element.items[0].children[0].name = 'New leaf';
    element.$update();
  });
  await expect(tree.locator('h2')).toHaveText('Updated forest');
  await expect(tree.locator('.name')).toHaveText(['Oak', 'New leaf', 'Bud', 'Pine']);
  await page.evaluate(() => {
    const element = document.querySelector('test-recursive-tree') as TestRecursiveTree;
    element.$update();
    element.$flushUpdates();
  });
  expect(await leaf.evaluate((node, before) => node === before, original)).toBe(true);
  await tree.locator('[data-id="leaf"] > .select').click();
  await expect(tree.locator('.selected')).toHaveText('leaf:New leaf');
});

test('nested keyed reorder retains focused inputs and live event scope', async ({ page }) => {
  const tree = page.locator('test-recursive-tree');
  const leaf = tree.locator('[data-id="leaf"] > input');
  await leaf.focus();
  const original = await leaf.elementHandle();
  await page.evaluate(() => {
    const element = document.querySelector('test-recursive-tree') as TestRecursiveTree;
    element.items = element.items.toReversed().map((item) => ({
      ...item,
      children: item.children.toReversed().map((child) => ({
        ...child,
        name: `${child.name}!`,
      })),
    }));
  });
  await expect(tree.locator('.name')).toHaveText(['Pine', 'Oak', 'Bud!', 'Leaf!']);
  expect(await leaf.evaluate((node, before) => node === before, original)).toBe(true);
  await expect(leaf).toBeFocused();
  await tree.locator('[data-id="leaf"] > .select').click();
  await expect(tree.locator('.selected')).toHaveText('leaf:Leaf!');
});

test('removal and reinsertion reconcile recursive subtrees without ghosts', async ({ page }) => {
  const tree = page.locator('test-recursive-tree');
  await page.evaluate(() => {
    const element = document.querySelector('test-recursive-tree') as TestRecursiveTree;
    element.items = [{ ...element.items[0], children: [] }];
  });
  await expect(tree.locator('.name')).toHaveText(['Oak']);
  await page.evaluate(() => {
    const element = document.querySelector('test-recursive-tree') as TestRecursiveTree;
    element.items = [{
      ...element.items[0],
      children: [{ id: 'new', name: 'New branch', children: [] }],
    }];
  });
  await expect(tree.locator('.name')).toHaveText(['Oak', 'New branch']);
  await tree.locator('[data-id="new"] > .select').click();
  await expect(tree.locator('.selected')).toHaveText('new:New branch');
});

test('client creation uses the same compiled graph and reconnect preserves current state', async ({ page }) => {
  await page.evaluate(() => {
    const element = document.createElement('test-recursive-tree') as TestRecursiveTree;
    element.id = 'client-tree';
    element.title = 'Client';
    element.items = [{
      id: 'client',
      name: 'Client root',
      children: [{ id: 'child', name: 'Client child', children: [] }],
    }];
    document.body.append(element);
  });
  const client = page.locator('#client-tree');
  await expect(client.locator('.name')).toHaveText(['Client root', 'Client child']);
  await page.evaluate(async () => {
    const element = document.querySelector('#client-tree') as TestRecursiveTree;
    element.remove();
    await new Promise<void>((resolve) => queueMicrotask(resolve));
    element.title = 'Reconnected';
    document.body.append(element);
  });
  await expect(client.locator('h2')).toHaveText('Reconnected');
  await expect(client.locator('.name')).toHaveText(['Client root', 'Client child']);
  expect(await client.evaluate((element: TestRecursiveTree) => element.hydrations)).toBe(1);
  await client.locator('[data-id="child"] > .select').click();
  await expect(client.locator('.selected')).toHaveText('child:Client child');
});

test('light DOM orders adjacent text, condition, empty call, call, and repeat slots', async ({ page }) => {
  const light = page.locator('test-recursive-light');
  await expect(light.locator('.adjacent')).toHaveText('BeforeForestAfterForestonetwoAfter');
  await page.evaluate(() => {
    const element = document.querySelector('test-recursive-light') as TestRecursiveLight;
    element.show = false;
    element.title = 'New';
    element.prefix = 'Start';
    element.suffix = 'End';
    element.tail = ['three'];
    element.html = '<u class="trusted">Replaced</u><strong>Second</strong>';
    element.scalar = 0;
  });
  await expect(light.locator('.adjacent')).toHaveText('StartEndNewthreeEnd');
  await expect(light.locator('.rich')).toHaveText('ReplacedSecondNew');
  await expect(light.locator('.scalar')).toHaveText('0');
  await page.evaluate(() => {
    const element = document.querySelector('test-recursive-light') as TestRecursiveLight;
    element.show = true;
    element.scalar = false;
  });
  await expect(light.locator('.adjacent')).toHaveText('StartNewEndNewthreeEnd');
  await expect(light.locator('.scalar')).toHaveText('false');
});

test('table render calls retain tbody parsing context through updates', async ({ page }) => {
  const table = page.locator('test-recursive-table');
  await expect(table.locator('.explicit > tbody > tr > td')).toHaveText(['Oak', 'Pine']);
  await page.evaluate(() => {
    const element = document.querySelector('test-recursive-table') as TestRecursiveTable;
    element.items = element.items.toReversed();
  });
  await expect(table.locator('.explicit > tbody > tr > td')).toHaveText(['Pine', 'Oak']);
});

test('implicit table bodies and column groups retain complete render ranges', async ({ page }) => {
  const table = page.locator('test-recursive-table');
  await expect(table.locator('.implicit > tbody > tr')).toHaveCount(4);
  await expect(table.locator('.implicit-row > td')).toHaveText(['Forest', 'Forest']);
  await expect(table.locator('.column-table > colgroup > col')).toHaveAttribute('span', '2');
  await page.evaluate(() => {
    const element = document.querySelector('test-recursive-table') as TestRecursiveTable;
    element.title = 'New';
    element.show = false;
    element.columns = 3;
    element.items = [{ id: 'new', name: 'Branch', children: [] }];
  });
  await expect(table.locator('.implicit > tbody > tr')).toHaveCount(2);
  await expect(table.locator('.implicit-row > td')).toHaveText('New');
  await expect(table.locator('.item-row > td')).toHaveText('BranchNewNew');
  await expect(table.locator('.column-table > colgroup > col')).toHaveAttribute('span', '3');
  await page.evaluate(() => {
    const element = document.querySelector('test-recursive-table') as TestRecursiveTable;
    element.show = true;
  });
  await expect(table.locator('.implicit > tbody > tr')).toHaveCount(3);
  await expect(table.locator('.item-row > td')).toHaveText('BranchNewNewNew');
});

test('string-length call inputs use UTF-8 bytes in SSR and reactive updates', async ({ page }) => {
  const length = page.locator('test-recursive-light .byte-length');
  await expect(length).toHaveText('6');
  for (const [text, bytes] of [
    ['é', '2'], ['😀', '4'], ['A😎', '5'], ['e\u0301', '3'], ['\ud800', '3'],
  ]) {
    await page.evaluate((text) => {
      const element = document.querySelector('test-recursive-light') as TestRecursiveLight;
      element.lengthText = text;
    }, text);
    await expect(length).toHaveText(bytes);
  }
});

test('a known synthetic length cannot be traversed by a render input', async ({ page }) => {
  const result = await page.evaluate(() => {
    const element = document.querySelector('test-recursive-light') as TestRecursiveLight;
    try {
      element.invalidLength = true;
      element.$flushUpdates();
      return '';
    } catch (error) {
      return String(error);
    }
  });
  expect(result).toContain('lengthText.length.more');
});

test('unknown template-only call inputs retain trusted SSR until explicitly supplied', async ({ page }) => {
  const unknown = page.locator('test-recursive-unknown');
  await expect(unknown.locator('.unknown-name')).toHaveText(['Server oak', 'Server leaf']);
  await unknown.locator('button').click();
  await expect(unknown.locator('button')).toHaveText('1');
  await expect(unknown.locator('h3')).toHaveText('Server-only forest');
  await expect(unknown.locator('.unknown-name')).toHaveText(['Server oak', 'Server leaf']);
  await page.evaluate(() => {
    const element = document.querySelector('test-recursive-unknown') as TestRecursiveUnknown;
    element.setState({
      serverTitle: 'Supplied',
      serverItems: [{ id: 'known', name: 'Known tree', children: [] }],
    });
  });
  await expect(unknown.locator('h3')).toHaveText('Supplied');
  await expect(unknown.locator('.unknown-name')).toHaveText(['Known tree']);
});

test('deferred hydration replays newer recursive state before handling the first event', async ({ page }) => {
  await page.addInitScript(() => {
    class DeferredObserver {
      observe(): void {}
      unobserve(): void {}
      disconnect(): void {}
    }
    Object.defineProperty(window, 'IntersectionObserver', { value: DeferredObserver });
  });
  await page.reload();
  await page.waitForFunction(() => customElements.get('test-recursive-lazy') !== undefined);
  const lazy = page.locator('test-recursive-lazy');
  expect(await lazy.evaluate((element: TestRecursiveTree) => element.hydrations)).toBe(0);
  await page.evaluate(() => {
    const element = document.querySelector('test-recursive-lazy') as TestRecursiveTree;
    element.title = 'Deferred';
    element.items = [{ id: 'late', name: 'Late tree', children: [] }];
    element.dispatchEvent(new PointerEvent('pointerover', { bubbles: true, composed: true }));
  });
  await expect(lazy.locator('h2')).toHaveText('Deferred');
  await expect(lazy.locator('.name')).toHaveText(['Late tree']);
  await lazy.locator('button').click();
  await expect(lazy.locator('.selected')).toHaveText('late:Late tree');
  expect(await lazy.evaluate((element: TestRecursiveTree) => element.hydrations)).toBe(1);
});
