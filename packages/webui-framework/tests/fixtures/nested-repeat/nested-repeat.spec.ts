// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('nested repeat fixture', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/nested-repeat/fixture.html');
    await page.waitForSelector('test-nested-repeat');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-nested-repeat');
      return el && (el as any).$ready === true;
    });
  });

  test('resolves outer scope values inside nested repeat items', async ({ page }) => {
    await page.locator('test-nested-repeat .load').click();

    await expect(page.locator('test-nested-repeat h2')).toHaveText(['Color', 'Size']);
    await expect(page.locator('test-nested-repeat .value')).toHaveText(['Black', 'Blue', 'S', 'M']);
    await expect(page.locator('test-nested-repeat .value').nth(1)).toBeDisabled();

    const groups = await page.locator('test-nested-repeat .value').evaluateAll((elements) => {
      return elements.map((element) => element.getAttribute('data-group'));
    });

    expect(groups).toEqual(['Color', 'Color', 'Size', 'Size']);
  });

  test('updating groups with new objects does not duplicate inner items', async ({ page }) => {
    await page.locator('test-nested-repeat .load').click();
    await expect(page.locator('test-nested-repeat .value')).toHaveCount(4);

    // Re-set groups with new objects (same data) — triggers nested reconciliation
    await page.evaluate(() => {
      (document.querySelector('test-nested-repeat') as any).updateGroups();
    });

    // Must still have exactly 4 inner items, not 8
    await expect(page.locator('test-nested-repeat .value')).toHaveCount(4);
    await expect(page.locator('test-nested-repeat .value')).toHaveText(['Black', 'Blue', 'S', 'M']);
  });

  test('updating groups multiple times does not accumulate duplicates', async ({ page }) => {
    await page.locator('test-nested-repeat .load').click();
    await expect(page.locator('test-nested-repeat .value')).toHaveCount(4);

    for (let i = 0; i < 5; i++) {
      await page.evaluate(() => {
        (document.querySelector('test-nested-repeat') as any).updateGroups();
      });
    }

    await expect(page.locator('test-nested-repeat .value')).toHaveCount(4);
    await expect(page.locator('test-nested-repeat .value')).toHaveText(['Black', 'Blue', 'S', 'M']);

    const groups = await page.locator('test-nested-repeat .value').evaluateAll((elements) => {
      return elements.map((element) => element.getAttribute('data-group'));
    });
    expect(groups).toEqual(['Color', 'Color', 'Size', 'Size']);
  });

  test('growing an inner list does not duplicate existing items', async ({ page }) => {
    await page.locator('test-nested-repeat .load').click();
    await expect(page.locator('test-nested-repeat .value')).toHaveCount(4);

    await page.evaluate(() => {
      (document.querySelector('test-nested-repeat') as any).growFirstGroup();
    });

    await expect(page.locator('test-nested-repeat .value')).toHaveCount(5);
    await expect(page.locator('test-nested-repeat .value')).toHaveText([
      'Black', 'Blue', 'Red', 'S', 'M',
    ]);
  });

  test('shrinking an inner list removes items correctly', async ({ page }) => {
    await page.locator('test-nested-repeat .load').click();
    await expect(page.locator('test-nested-repeat .value')).toHaveCount(4);

    await page.evaluate(() => {
      (document.querySelector('test-nested-repeat') as any).shrinkFirstGroup();
    });

    await expect(page.locator('test-nested-repeat .value')).toHaveCount(3);
    await expect(page.locator('test-nested-repeat .value')).toHaveText(['Blue', 'S', 'M']);
  });
});

// ── SSR hydration regression (#175 / #176) ──────────────────────
// Exercises $resolveSSR and $resolve with pathStart > 0 on paths
// deeper than the block root (e.g. [0, 1]).  Before the fix, the
// template cursor was not advanced through skipped path segments,
// so inner repeats failed to find their parent element and markers.

test.describe('nested repeat SSR hydration', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/nested-repeat/fixture.html');
    await page.waitForSelector('test-nested-repeat');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-nested-repeat');
      return el && (el as any).$ready === true;
    });
  });

  test('inner repeat items are hydrated without duplication', async ({ page }) => {
    // SSR HTML contains 4 buttons; after hydration they must remain exactly 4.
    await expect(page.locator('test-nested-repeat .value')).toHaveCount(4);
    await expect(page.locator('test-nested-repeat .value')).toHaveText([
      'Black', 'Blue', 'S', 'M',
    ]);
  });

  test('inner repeat items preserve attributes from SSR', async ({ page }) => {
    const groups = await page.locator('test-nested-repeat .value').evaluateAll(
      (els) => els.map((el) => el.getAttribute('data-group')),
    );
    expect(groups).toEqual(['Color', 'Color', 'Size', 'Size']);

    // The disabled attribute on "Blue" must survive hydration
    await expect(page.locator('test-nested-repeat .value').nth(1)).toBeDisabled();
    await expect(page.locator('test-nested-repeat .value').nth(0)).toBeEnabled();
  });

  test('reactive update after SSR hydration does not duplicate inner items', async ({ page }) => {
    await expect(page.locator('test-nested-repeat .value')).toHaveCount(4);

    // Trigger a reactive update with new object references
    await page.evaluate(() => {
      (document.querySelector('test-nested-repeat') as any).updateGroups();
    });

    // Must still be exactly 4 — no SSR ghosts left behind
    await expect(page.locator('test-nested-repeat .value')).toHaveCount(4);
    await expect(page.locator('test-nested-repeat .value')).toHaveText([
      'Black', 'Blue', 'S', 'M',
    ]);
  });

  test('growing inner list after SSR hydration works correctly', async ({ page }) => {
    await expect(page.locator('test-nested-repeat .value')).toHaveCount(4);

    await page.evaluate(() => {
      (document.querySelector('test-nested-repeat') as any).growFirstGroup();
    });

    await expect(page.locator('test-nested-repeat .value')).toHaveCount(5);
    await expect(page.locator('test-nested-repeat .value')).toHaveText([
      'Black', 'Blue', 'Red', 'S', 'M',
    ]);
  });

  test('outer scope text bindings hydrate correctly', async ({ page }) => {
    await expect(page.locator('test-nested-repeat h2')).toHaveText(['Color', 'Size']);
  });

  test('innermost keyed repeat hydrates and reorders through nested directives', async ({ page }) => {
    await page.waitForFunction(() => {
      const element = document.querySelector('test-nested-repeat-keyed-chain');
      return element && (element as any).$ready === true;
    });

    const items = page.locator('test-nested-repeat-keyed-chain .keyed-chain-item');
    await expect(items).toHaveCount(4);
    await expect(items).toHaveText(['G1 A', 'G1 B', 'G2 A', 'G2 C']);

    await page.evaluate(() => {
      const parent = document.querySelector(
        'test-nested-repeat-keyed-chain',
      ) as HTMLElement & { reverseItems(): void };
      const elements = parent.shadowRoot?.querySelectorAll('.keyed-chain-item');
      const nodes = new Map<string, Element>();
      elements?.forEach((element) => {
        nodes.set(
          `${element.getAttribute('data-group')}:${element.getAttribute('data-id')}`,
          element,
        );
      });
      (window as unknown as { __keyedChainNodes?: Map<string, Element> })
        .__keyedChainNodes = nodes;
      parent.reverseItems();
    });

    await expect(items).toHaveText([
      'G1 B updated',
      'G1 A updated',
      'G2 C updated',
      'G2 A updated',
    ]);
    const preserved = await page.evaluate(() => {
      const parent = document.querySelector('test-nested-repeat-keyed-chain');
      const elements = parent?.shadowRoot?.querySelectorAll('.keyed-chain-item');
      const nodes = (window as unknown as {
        __keyedChainNodes?: Map<string, Element>;
      }).__keyedChainNodes;
      return Array.from(elements ?? []).every((element) => (
        nodes?.get(
          `${element.getAttribute('data-group')}:${element.getAttribute('data-id')}`,
        ) === element
      ));
    });

    expect(preserved).toBe(true);
  });
});

test.describe('sibling repeat after root-level nested repeat (#405)', () => {
  test('hydrates the second repeat against its own marker', async ({ page }) => {
    await page.goto('/nested-repeat/fixture.html');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-repeat-siblings');
      return el && (el as unknown as { $ready?: boolean }).$ready === true;
    });

    const host = page.locator('test-repeat-siblings');
    const others = host.locator('.other');

    await expect(others).toHaveCount(2);
    await expect(others).toHaveText(['One', 'Two']);

    await others.first().click();
    await expect(host.locator('.selected')).toHaveText('One');

    await page.evaluate(() => {
      (document.querySelector('test-repeat-siblings') as HTMLElement & {
        replaceOthers(): void;
      }).replaceOthers();
    });

    await expect(others).toHaveCount(2);
    await expect(others).toHaveText(['Three', 'Four']);
  });
});

test.describe('repeat that revisits an earlier parent (#431)', () => {
  test('hydrates the trailing repeat against its own marker', async ({ page }) => {
    await page.goto('/nested-repeat/fixture.html');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-repeat-interleaved');
      return el && (el as unknown as { $ready?: boolean }).$ready === true;
    });

    const host = page.locator('test-repeat-interleaved');

    await expect(host.locator('.head-item')).toHaveText(['H1', 'H2']);
    await expect(host.locator('.panel-head .inner-item')).toHaveText(['I1', 'I2']);
    await expect(host.locator('.tail-item')).toHaveText(['T1', 'T2']);

    await page.evaluate(() => {
      (document.querySelector('test-repeat-interleaved') as HTMLElement & {
        replaceTail(): void;
      }).replaceTail();
    });

    await expect(host.locator('.tail-item')).toHaveText(['T3', 'T4', 'T5']);
    await expect(host.locator('.head-item')).toHaveText(['H1', 'H2']);
    await expect(host.locator('.panel-head .inner-item')).toHaveText(['I1', 'I2']);
  });
});

test.describe('repeat after a root-level conditional-nested repeat', () => {
  test('hydrates the trailing repeat against its own marker', async ({ page }) => {
    await page.goto('/nested-repeat/fixture.html');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-repeat-after-conditional');
      return el && (el as unknown as { $ready?: boolean }).$ready === true;
    });

    const host = page.locator('test-repeat-after-conditional');

    await expect(host.locator('.nested-row')).toHaveText(['X1', 'X2']);
    await expect(host.locator('.nested-label')).toHaveText('label');
    await expect(host.locator('.tail-row')).toHaveText(['Y1', 'Y2']);

    await page.evaluate(() => {
      (document.querySelector('test-repeat-after-conditional') as HTMLElement & {
        replaceTailRows(): void;
      }).replaceTailRows();
    });

    // The trailing repeat owns its own <!--wr--> range; the repeat nested
    // inside the conditional branch must be untouched.
    await expect(host.locator('.tail-row')).toHaveText(['Y3', 'Y4', 'Y5']);
    await expect(host.locator('.nested-row')).toHaveText(['X1', 'X2']);
    await expect(host.locator('.nested-label')).toHaveText('label');
  });
});
