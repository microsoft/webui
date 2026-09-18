// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test, type Locator } from '@playwright/test';

test('outlet-only work preserves unknown SSR item scopes during known-root updates', async ({ page }) => {
  await page.goto('/outlet-hydration/fixture.html');
  const shell = page.locator('test-outlet-unknown');
  await expect(shell.locator('.item')).toHaveText('server-item/right');
  await shell.getByRole('button', { name: 'Update', exact: true }).click();
  await expect(shell.locator('.item')).toHaveText('server-item/right');
  await expect(shell.locator('.item')).toHaveAttribute('title', 'server-item/right');
  await shell.getByRole('button', { name: 'Toggle primary' }).click();
  await expect(shell.locator('.detail')).toHaveText('server-item');
  expect(await shell.evaluate(node => Reflect.get(node, '$fragmentKnownRoots'))).toBeUndefined();
});

test('outlet-only reconnect preserves unavailable root state', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto('/outlet-hydration/fixture.html');
  const shell = page.locator('test-outlet-unknown');
  await expect.poll(() => shell.evaluate(node => Reflect.get(node, '$ready'))).toBe(true);
  await shell.evaluate(async node => {
    const parent = node.parentNode!;
    const next = node.nextSibling;
    node.remove();
    await new Promise(resolve => setTimeout(resolve, 0));
    parent.insertBefore(node, next);
  });
  await expect.poll(() => shell.evaluate(node => Reflect.get(node, '$ready'))).toBe(true);
  await expect(shell.locator('.root-snapshot')).toHaveText('server-label/right');
  await expect(shell.locator('.item')).toHaveText('server-item/right');
  expect(errors).toEqual([]);
});

function textSlots(element: Locator): Promise<string[]> {
  return element.evaluate(node => Array.from(node.childNodes)
    .filter(child => child.nodeType === Node.TEXT_NODE)
    .map(child => child.textContent ?? '')
    .filter(value => value.trim().length > 0));
}

test('hydrates route expansions and independent text slots around empty outlets', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto('/outlet-hydration/fixture.html');
  const shell = page.locator('test-outlet-shell');
  await expect.poll(() => textSlots(shell.locator('.populated'))).toEqual(['left', 'right']);
  expect(await shell.locator('.raw-before').evaluate(node => Array.from(node.childNodes)
    .filter(child => child.nodeType === Node.COMMENT_NODE)
    .map(child => child.nodeValue))).toEqual(['w0', 'w0', '/w0', 'wo', '/wo', '/w0']);
  await expect(shell.locator('.empty')).toHaveText('empty-leftempty-right');
  await expect(shell.locator('.fragment')).toHaveText('fragment-leftfragment-right');
  await expect(shell.locator('webui-route')).toHaveCount(2);
  const leaf = shell.locator('test-outlet-leaf');
  await expect(leaf.locator('.leaf')).toHaveText('Route content');
  const original = await leaf.elementHandle();

  await shell.getByRole('button', { name: 'Update' }).click();
  await expect.poll(() => textSlots(shell.locator('.populated'))).toEqual(['next-left', 'next-right']);
  await expect(shell.locator('.empty')).toHaveText('next-empty-leftnext-empty-right');
  await expect(shell.locator('.fragment')).toHaveText('next-fragment-leftnext-fragment-right');
  expect(await leaf.evaluate((node, previous) => node === previous, original)).toBe(true);
  expect(errors).toEqual([]);
});

for (const empty of [false, true]) {
  test(`inserts a new route inside the ${empty ? 'empty' : 'populated'} SSR outlet`, async ({ page }) => {
    const errors: string[] = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto('/outlet-hydration/fixture.html');
    const shell = page.locator('test-outlet-shell');
    await expect(shell.locator('test-outlet-leaf .leaf')).toHaveText('Route content');
    if (empty) {
      await shell.locator('.populated').evaluate(node => {
        for (const route of node.querySelectorAll('webui-route')) route.remove();
      });
    }

    await shell.getByRole('button', { name: 'Add route' }).click();
    const positions = await shell.locator('.populated').evaluate(node => {
      const children = Array.from(node.childNodes);
      return {
        start: children.findIndex(child => child.nodeType === Node.COMMENT_NODE && child.nodeValue === 'wo'),
        route: children.findIndex(child => child instanceof Element && child.getAttribute('path') === 'added'),
        end: children.findIndex(child => child.nodeType === Node.COMMENT_NODE && child.nodeValue === '/wo'),
      };
    });
    expect(positions.start).toBeGreaterThanOrEqual(0);
    expect(positions.route).toBeGreaterThan(positions.start);
    expect(positions.route).toBeLessThan(positions.end);
    await shell.getByRole('button', { name: 'Update' }).click();
    await expect.poll(() => textSlots(shell.locator('.populated'))).toEqual(['next-left', 'next-right']);
    expect(errors).toEqual([]);
  });
}

test('a recreated client outlet wins over later SSR outlets and releases newly inserted routes', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto('/outlet-hydration/fixture.html');
  const shell = page.locator('test-outlet-shell');
  await expect(shell.locator('test-outlet-leaf .leaf')).toHaveText('Route content');
  const toggle = shell.getByRole('button', { name: 'Toggle primary' });
  await toggle.click();
  await expect(shell.locator('.populated webui-route')).toHaveCount(0);
  await toggle.click();
  await expect.poll(() => shell.locator('.populated').evaluate(node => Array.from(node.childNodes)
    .filter(child => child.nodeType === Node.COMMENT_NODE && child.nodeValue === 'wo').length)).toBe(1);
  await expect(shell.locator('.populated outlet')).toHaveCount(0);
  await shell.getByRole('button', { name: 'Add route' }).click();
  await expect(shell.locator('.populated webui-route[path="added"]')).toHaveCount(1);
  await expect(shell.locator('.empty webui-route')).toHaveCount(0);
  await shell.getByRole('button', { name: 'Update' }).click();
  await expect.poll(() => textSlots(shell.locator('.populated'))).toEqual(['next-left', 'next-right']);
  await toggle.click();
  await expect(shell.locator('webui-route[path="added"]')).toHaveCount(0);
  expect(errors).toEqual([]);
});

test('non-fragment outlet blocks retain inserted routes without capturing fragment state', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto('/outlet-hydration/fixture.html');
  const shell = page.locator('test-outlet-plain');
  await expect(shell.locator('.populated')).toHaveText('leftright');
  expect(await shell.evaluate(node => Reflect.get(node, '$fragmentKnownRoots'))).toBeUndefined();
  const toggle = shell.getByRole('button', { name: 'Toggle primary' });
  for (let i = 0; i < 2; i++) {
    await shell.getByRole('button', { name: 'Add route' }).click();
    await expect(shell.locator('webui-route[path="added"]')).toHaveCount(1);
    await toggle.click();
    await expect(shell.locator('webui-route[path="added"]')).toHaveCount(0);
    await toggle.click();
    await expect.poll(() => shell.locator('.populated').evaluate(node => Array.from(node.childNodes)
      .filter(child => child.nodeType === Node.COMMENT_NODE && child.nodeValue === 'wo').length)).toBe(1);
  }
  expect(await shell.evaluate(node => Reflect.get(node, '$fragmentKnownRoots'))).toBeUndefined();
  expect(await shell.evaluate(node => Reflect.get(node, '$fragmentInputVersions'))).toBeUndefined();
  expect(errors).toEqual([]);
});
