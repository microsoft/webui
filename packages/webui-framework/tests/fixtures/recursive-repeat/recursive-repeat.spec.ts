// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';
import type { TestRecursiveTree } from './element.js';

test.beforeEach(async ({ page }) => {
  await page.goto('/recursive-repeat/fixture.html');
  await page.waitForFunction(() => (
    ['tree', 'forward', 'independent', 'mutual'].every((name) => (
      (document.querySelector(`test-recursive-${name}`) as any)?.$ready === true
    ))
  ));
});

test('hydrates a cyclic block table without duplicating recursive DOM', async ({ page }) => {
  await expect(page.locator('test-recursive-tree .name')).toHaveText([
    'Root', 'Branch', 'Leaf', 'Empty', 'Sibling',
  ]);
  await expect(page.locator('test-recursive-tree .after')).toHaveText([
    'Leaf', 'Branch', 'Empty', 'Root', 'Sibling',
  ]);
  await expect(page.locator('test-recursive-tree .outer')).toHaveText('Component scope');
  const blocks = await page.evaluate(() => {
    const meta = (window as any).__webui.templates['test-recursive-tree'];
    return { count: meta.b.length, root: meta.r[0][2], recursive: meta.b[1].r[0][2] };
  });
  expect(blocks).toEqual({ count: 2, root: 0, recursive: 0 });
});

test('renders finite recursive trees before browser JavaScript runs', async ({ browser, baseURL }) => {
  const context = await browser.newContext({ baseURL, javaScriptEnabled: false });
  try {
    const page = await context.newPage();
    await page.goto('/recursive-repeat/fixture.html');
    await expect(page.locator('test-recursive-tree .name')).toHaveText([
      'Root', 'Branch', 'Leaf', 'Empty', 'Sibling',
    ]);
    await expect(page.locator('test-recursive-tree .outer')).toHaveText('Component scope');
    await expect(page.locator('test-recursive-forward .forward-name')).toHaveText([
      'Forward root', 'Forward leaf',
    ]);
    await expect(page.locator('test-recursive-mutual .mutual-name')).toHaveText([
      'Left root', 'Right child', 'Left leaf',
    ]);
  } finally {
    await context.close();
  }
});

test('recursive events retain the current item and restore outer item scopes', async ({ page }) => {
  const tree = page.locator('test-recursive-tree');
  await tree.locator('li[data-id="leaf"] > .select').click();
  await expect(tree.locator('.selected')).toHaveText('leaf');
  await tree.locator('li[data-id="root"] > .after').click();
  await expect(tree.locator('.selected')).toHaveText('root');
  await tree.locator('li[data-id="sibling"] > .select').click();
  await expect(tree.locator('.selected')).toHaveText('sibling');
  await expect(tree.locator('.clicks')).toHaveText('3');
});

test('keyed recursive updates preserve identity, update handlers and prune missing children', async ({ page }) => {
  await page.evaluate(() => {
    const tree = document.querySelector('test-recursive-tree') as TestRecursiveTree;
    const root = tree.shadowRoot!;
    (window as any).recursiveNodes = ['root', 'branch', 'leaf', 'empty', 'sibling']
      .map((id) => root.querySelector(`li[data-id="${id}"]`));
    tree.items = [
      { id: 'sibling', name: 'Sibling updated' },
      { id: 'root', name: 'Root updated', children: [
        { id: 'empty', name: 'Empty updated', children: [] },
        { id: 'branch', name: 'Branch updated', children: [
          { id: 'leaf', name: 'Leaf updated' },
          { id: 'new', name: 'New child' },
        ] },
      ] },
    ];
    tree.prefix = 'Updated: ';
  });
  const tree = page.locator('test-recursive-tree');
  await expect(tree.locator('.name')).toHaveText([
    'Sibling updated', 'Root updated', 'Empty updated', 'Branch updated', 'Leaf updated', 'New child',
  ]);
  expect(await page.evaluate(() => {
    const root = document.querySelector('test-recursive-tree')!.shadowRoot!;
    return ['root', 'branch', 'leaf', 'empty', 'sibling'].every((id, index) => (
      root.querySelector(`li[data-id="${id}"]`) === (window as any).recursiveNodes[index]
    ));
  })).toBe(true);
  await expect(tree.locator('.prefix')).toHaveText(Array(6).fill('Updated: '));
  await tree.locator('li[data-id="new"] > .select').click();
  await expect(tree.locator('.selected')).toHaveText('new');

  await page.evaluate(() => {
    const tree = document.querySelector('test-recursive-tree') as TestRecursiveTree;
    tree.items = [{ id: 'root', name: 'Root without children' }];
  });
  await expect(tree.locator('.name')).toHaveText(['Root without children']);
  await expect(tree.locator('.after')).toHaveText(['Root without children']);
  await expect(tree.locator('.outer')).toHaveText('Component scope');
  await page.evaluate(() => {
    (document.querySelector('test-recursive-tree') as TestRecursiveTree).items = [];
  });
  await expect(tree.locator('li')).toHaveCount(0);
});

test('forward references, mutual references and component-local names stay independent', async ({ page }) => {
  await expect(page.locator('test-recursive-forward .forward-name')).toHaveText([
    'Forward root', 'Forward leaf',
  ]);
  await page.locator('test-recursive-forward .forward-name').nth(1).click();
  await expect(page.locator('test-recursive-forward .forward-selected')).toHaveText('forward-leaf');
  await expect(page.locator('test-recursive-independent .independent-name')).toHaveText([
    'Independent root', 'Independent leaf',
  ]);
  await expect(page.locator('test-recursive-mutual .mutual-name')).toHaveText([
    'Left root', 'Right child', 'Left leaf',
  ]);
  expect(await page.locator('test-recursive-mutual .mutual-name').evaluateAll(
    (nodes) => nodes.map((node) => node.getAttribute('data-side')),
  )).toEqual(['left', 'right', 'left']);
});

test('client-created instances render recursion and react without SSR markers', async ({ page }) => {
  await page.evaluate(() => {
    const tree = document.createElement('test-recursive-tree') as TestRecursiveTree;
    tree.id = 'client-tree';
    tree.items = [{
      id: 'client-root', name: 'Client root', children: [
        { id: 'client-leaf', name: 'Client leaf', children: [] },
      ],
    }];
    document.body.appendChild(tree);
  });
  const tree = page.locator('#client-tree');
  await expect(tree.locator('.name')).toHaveText(['Client root', 'Client leaf']);
  await tree.locator('li[data-id="client-leaf"] > .select').click();
  await expect(tree.locator('.selected')).toHaveText('client-leaf');
  await page.evaluate(() => {
    (document.querySelector('#client-tree') as TestRecursiveTree).items = [{
      id: 'client-root', name: 'Client root changed', children: [
        { id: 'second', name: 'Second', children: [
          { id: 'third', name: 'Third' },
        ] },
      ],
    }];
  });
  await expect(tree.locator('.name')).toHaveText(['Client root changed', 'Second', 'Third']);
  await tree.locator('li[data-id="client-root"] > .after').click();
  await expect(tree.locator('.selected')).toHaveText('client-root');
  await expect(tree.locator('.outer')).toHaveText('Component scope');
});
