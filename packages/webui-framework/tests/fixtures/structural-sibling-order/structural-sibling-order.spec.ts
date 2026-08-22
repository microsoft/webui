// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test, type Locator } from '@playwright/test';

async function directChildKinds(section: Locator): Promise<string[]> {
  return section.locator(':scope > [data-kind]').evaluateAll(
    (elements) => elements.map((element) => element.getAttribute('data-kind') ?? ''),
  );
}

test.describe('client-created structural sibling order', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/structural-sibling-order/fixture.html');
    await page.waitForFunction(() => {
      const element = document.querySelector('test-structural-sibling-order');
      return element && (element as unknown as { $ready?: boolean }).$ready === true;
    });
  });

  test('matches SSR order for the keyed nested consumer shape', async ({ page }) => {
    const turns = page.locator('test-structural-sibling-order .exact-turn');

    await expect(turns).toHaveCount(1);
    expect(await directChildKinds(turns.first())).toEqual(['user', 'assistant', 'error']);

    await page.evaluate(() => {
      (document.querySelector('test-structural-sibling-order') as HTMLElement & {
        appendExactTerminal(): void;
      }).appendExactTerminal();
    });

    await expect(turns).toHaveCount(2);
    expect(await directChildKinds(turns.nth(1))).toEqual(['user', 'assistant', 'error']);
  });

  test('needs neither keys nor nested conditionals nor multiple assignments', async ({ page }) => {
    const turns = page.locator('test-structural-sibling-order .minimal-turn');

    await expect(turns).toHaveCount(1);
    expect(await directChildKinds(turns.first().locator('.for-if'))).toEqual([
      'user',
      'assistant',
      'error',
    ]);
    expect(await directChildKinds(turns.first().locator('.if-for'))).toEqual([
      'error',
      'user',
      'assistant',
    ]);

    await page.evaluate(() => {
      (document.querySelector('test-structural-sibling-order') as HTMLElement & {
        appendMinimalTerminal(): void;
      }).appendMinimalTerminal();
    });

    await expect(turns).toHaveCount(2);
    expect(await directChildKinds(turns.nth(1).locator('.for-if'))).toEqual([
      'user',
      'assistant',
      'error',
    ]);
    expect(await directChildKinds(turns.nth(1).locator('.if-for'))).toEqual([
      'error',
      'user',
      'assistant',
    ]);
  });

  test('keeps a conditional latent after an existing repeat', async ({ page }) => {
    const turns = page.locator('test-structural-sibling-order .minimal-turn');
    await page.evaluate(() => {
      (document.querySelector('test-structural-sibling-order') as HTMLElement & {
        appendMinimalTurn(messages: string[], failed: boolean): void;
      }).appendMinimalTurn(['user', 'assistant'], false);
    });

    await expect(turns).toHaveCount(2);
    expect(await directChildKinds(turns.nth(1).locator('.for-if'))).toEqual([
      'user',
      'assistant',
    ]);

    await page.evaluate(() => {
      (document.querySelector('test-structural-sibling-order') as HTMLElement & {
        updateLastMinimalTurn(patch: { failed: boolean }): void;
      }).updateLastMinimalTurn({ failed: true });
    });

    expect(await directChildKinds(turns.nth(1).locator('.for-if'))).toEqual([
      'user',
      'assistant',
      'error',
    ]);
    expect(await directChildKinds(turns.nth(1).locator('.if-for'))).toEqual([
      'error',
      'user',
      'assistant',
    ]);
  });

  test('keeps a late-growing repeat before an existing conditional', async ({ page }) => {
    const turns = page.locator('test-structural-sibling-order .minimal-turn');
    await page.evaluate(() => {
      (document.querySelector('test-structural-sibling-order') as HTMLElement & {
        appendMinimalTurn(messages: string[], failed: boolean): void;
      }).appendMinimalTurn([], true);
    });

    await expect(turns).toHaveCount(2);
    expect(await directChildKinds(turns.nth(1).locator('.for-if'))).toEqual([
      'error',
    ]);

    await page.evaluate(() => {
      (document.querySelector('test-structural-sibling-order') as HTMLElement & {
        updateLastMinimalTurn(patch: { messages: string[] }): void;
      }).updateLastMinimalTurn({ messages: ['user', 'assistant'] });
    });

    expect(await directChildKinds(turns.nth(1).locator('.for-if'))).toEqual([
      'user',
      'assistant',
      'error',
    ]);
    expect(await directChildKinds(turns.nth(1).locator('.if-for'))).toEqual([
      'error',
      'user',
      'assistant',
    ]);
  });

  test('hydrates an empty text slot before a following repeat', async ({ page }) => {
    const container = page.locator('test-structural-sibling-order .text-before-repeat');
    await expect(container).toHaveText('itemtail');

    await page.evaluate(() => {
      (document.querySelector('test-structural-sibling-order') as HTMLElement & {
        setBeforeRepeat(): void;
      }).setBeforeRepeat();
    });

    await expect(container).toHaveText('beforeitemtail');
  });

  test('hydrates an empty text slot between conditional and repeat blocks', async ({ page }) => {
    const container = page.locator('test-structural-sibling-order .text-between-blocks');
    await expect(container).toHaveText('ifitemtail');

    await page.evaluate(() => {
      (document.querySelector('test-structural-sibling-order') as HTMLElement & {
        setBlockTexts(): void;
      }).setBlockTexts();
    });

    await expect(container).toHaveText('beforeifbetweenitemtail');
  });
});
