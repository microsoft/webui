// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';
import type { TestSuccessorComments, TestSuccessorPlain } from './element.js';

test('real compiler successors preserve absent ranges, static siblings, tables, and first updates', async ({ page }) => {
  await page.goto('/compiled-successors/fixture.html');
  const plain = page.locator('test-successor-plain'), calls = page.locator('test-successor-calls');
  await expect(plain.locator('.mixed')).toHaveText('ABCrawDstatic');
  await expect(calls.locator('.mixed')).toHaveText('ABVpieceCrawDstatic');
  const strong = await plain.locator('strong').elementHandle();
  const nested = await plain.locator('section > span').elementHandle();
  await page.evaluate(() => {
    for (const name of ['test-successor-plain', 'test-successor-calls']) {
      const element = document.querySelector(name) as TestSuccessorPlain;
      element.prefix = 'a';
      element.between = 'b';
      element.rawPrefix = 'c';
      element.staticPrefix = 'd';
      element.tail = 't';
      element.value = 'v';
      element.show = true;
      element.items = ['x', 'y'];
      element.html = '<u>new</u>';
    }
  });
  await expect(plain.locator('.mixed')).toHaveText('avbxycnewdstatict');
  await expect(calls.locator('.mixed')).toHaveText('abvpiecetcnewdstatict');
  await expect(plain.locator('tbody > tr > td')).toHaveText('acellt');
  await expect(plain.locator('section')).toHaveText('avnestedtt');
  await expect(calls.locator('section')).toHaveText('avpiecett');
  expect(await plain.locator('strong').evaluate((node, original) => node === original, strong)).toBe(true);
  expect(await plain.locator('section > span').evaluate((node, original) => node === original, nested)).toBe(true);
});

for (const tag of ['test-successor-comments', 'test-successor-fragment-comments']) {
  test(`${tag} preserves authored blank separators through hydration, writes, and reconnect`, async ({ page }) => {
    await page.goto('/compiled-successors/comments.html');
    const element = page.locator(tag);
    const span = element.locator('span');
    await expect(span).toHaveText('before');
    const original = await span.elementHandle();
    expect(await span.evaluate(node => Array.from(node.childNodes, child => [child.nodeType, child.textContent])))
      .toEqual([[3, 'before'], [8, ''], [3, '']]);
    await element.evaluate((node: TestSuccessorComments) => {
      node.prefix = 'first';
      node.suffix = 'last';
      node.root = 'changed';
    });
    await expect(span).toHaveText('firstlast');
    await expect(element).toHaveText('firstlastchanged');
    expect(await span.evaluate(node => Array.from(node.childNodes, child => [child.nodeType, child.textContent])))
      .toEqual([[3, 'first'], [8, ''], [3, 'last']]);
    await element.evaluate((node: TestSuccessorComments) => {
      node.remove();
      document.body.append(node);
      node.prefix = '';
      node.suffix = 'only';
    });
    await expect(span).toHaveText('only');
    expect(await span.evaluate((node, before) => node === before, original)).toBe(true);
  });
}
