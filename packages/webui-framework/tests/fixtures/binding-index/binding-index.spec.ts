// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';
import type { TestBindingIndex } from './element.js';

test.beforeEach(async ({ page }) => {
  await page.goto('/binding-index/fixture.html');
});

test('evaluates each distinct binding once per observable root, preserving repeat scopes', async ({ page }) => {
  await page.getByRole('button', { name: 'Rename', exact: true }).click();
  const host = page.locator('test-binding-index');
  await expect(host.locator('.double')).toHaveText('Grace Hopper');
  await expect(host.locator('.quad')).toHaveText('Grace Hopper Grace Hopper');
  await expect(host.locator('.eight')).toHaveText('Grace Hopper Grace Hopper Grace Hopper Grace Hopper');
  await expect(host.locator('.named')).toHaveAttribute('title', 'Grace Hopper');
  await expect(host.locator('.ready')).toHaveAttribute('data-ready', '');
  await expect(host.locator('.conditional')).toHaveText('Visible');
  await expect(host.locator('.row')).toHaveText(['Grace one Grace', 'Grace two Grace']);
  await expect(host.locator('.local')).toHaveText('Local record');
  await expect(host.locator('.wildcard')).toHaveText('External outside updated');
  const state = await host.evaluate((element: TestBindingIndex) => ({
    reads: element.reads, callbacks: element.callbacks,
  }));
  expect(state).toEqual({
    reads: {
      'record.first': 13, 'record.last': 9, suffix: 1,
      'item.label': 2, 'record.active': 2, 'record.allowed': 2,
      marker: 1, 'external.first': 1, extra: 1, 'external.last': 1,
    },
    callbacks: ['record'],
  });
});

test('retains each root of a mixed binding and the wildcard bucket', async ({ page }) => {
  await page.getByRole('button', { name: 'Change suffix', exact: true }).click();
  const host = page.locator('test-binding-index');
  await expect(host.locator('.mixed')).toHaveText('Ada updated Lovelace');
  await expect(host.locator('.double')).toHaveText('Ada Lovelace');
  expect(await host.evaluate((element: TestBindingIndex) => element.reads)).toEqual({
    'record.first': 1, suffix: 1, 'record.last': 1,
    'external.first': 1, extra: 1, 'external.last': 1,
  });
});

test('preserves reentrant dirty roots and authored callback ordering', async ({ page }) => {
  await page.getByRole('button', { name: 'Rename with callback', exact: true }).click();
  const host = page.locator('test-binding-index');
  await expect(host.locator('.mixed')).toHaveText('Grace reviewed Hopper');
  const state = await host.evaluate((element: TestBindingIndex) => ({
    firstReads: element.reads['record.first'],
    suffixReads: element.reads.suffix,
    callbacks: element.callbacks,
  }));
  expect(state).toEqual({ firstReads: 14, suffixReads: 2, callbacks: ['record', 'suffix'] });
});

test('keeps local shadowing out of the global root index', async ({ page }) => {
  await page.getByRole('button', { name: 'Change local records', exact: true }).click();
  const host = page.locator('test-binding-index');
  await expect(host.locator('.local')).toHaveText('Changed local');
  await expect(host.locator('.double')).toHaveText('Ada Lovelace');
  expect(await host.evaluate((element: TestBindingIndex) => element.reads)).toEqual({
    locals: 1, 'record.first': 1, 'record.last': 1,
    'external.first': 1, extra: 1, 'external.last': 1,
  });
});
