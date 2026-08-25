// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test, type Locator } from '@playwright/test';

async function directText(locator: Locator): Promise<string> {
  return locator.evaluate((element) => {
    let text = '';
    let node = element.firstChild;
    while (node) {
      if (node.nodeType === Node.TEXT_NODE) {
        text += node.textContent?.trim() ?? '';
      }
      node = node.nextSibling;
    }
    return text;
  });
}

test.describe('text-only repeat fixture', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/text-only-repeat/fixture.html');
    await page.waitForSelector('test-text-only-repeat');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-text-only-repeat');
      return el && (el as any).$ready === true;
    });
  });

  test('SSR renders only the active option label', async ({ page }) => {
    // After hydration, the label should show exactly "Relevance" — not duplicated
    const label = page.locator('test-text-only-repeat .label');
    await expect(label).toHaveText('Relevance');

    // Verify no duplication — text content should be exactly "Relevance"
    const text = await label.textContent();
    expect(text?.trim()).toBe('Relevance');
    await expect(page.locator('test-text-only-repeat .adjacent-label')).toHaveText('Relevance');
    expect(await directText(page.locator('test-text-only-repeat .adjacent-repeats')))
      .toBe('Relevance');
  });

  test('updating options does not duplicate the active label', async ({ page }) => {
    // Call onUpdate directly to avoid event wiring issues in this test
    await page.evaluate(() => {
      (document.querySelector('test-text-only-repeat') as any).onUpdate();
    });

    const label = page.locator('test-text-only-repeat .label');
    // Should show exactly "Trending" — not "RelevanceTrending"
    await expect(label).toHaveText('Trending');
    await expect(page.locator('test-text-only-repeat .adjacent-label')).toHaveText('Trending');
    expect(await directText(page.locator('test-text-only-repeat .adjacent-repeats')))
      .toBe('Trending');
  });

  test('only one active label is visible at a time', async ({ page }) => {
    // Count .active-label elements — should be exactly 1
    const count = await page.locator('test-text-only-repeat .active-label').count();
    expect(count).toBe(1);

    // After update, still exactly 1
    await page.evaluate(() => {
      (document.querySelector('test-text-only-repeat') as any).onUpdate();
    });
    const countAfter = await page.locator('test-text-only-repeat .active-label').count();
    expect(countAfter).toBe(1);
  });

  test('keeps only repeat anchors and releases removed item anchors', async ({ page }) => {
    const result = await page.evaluate(async () => {
      const host = document.querySelector('test-text-only-repeat') as any;
      host.options = Array.from({ length: 128 }, (_, index) => ({
        title: `Option ${index}`,
        active: true,
      }));
      await Promise.resolve();
      const root = host.shadowRoot ?? host;
      const comments = (): { repeats: number; conditions: Comment[] } => {
        let repeats = 0;
        const conditions: Comment[] = [];
        const walker = document.createTreeWalker(root, NodeFilter.SHOW_COMMENT);
        let node: Node | null;
        while ((node = walker.nextNode())) {
          const comment = node as Comment;
          if (comment.data === 'wr') repeats++;
          else if (comment.data === '' || comment.data === 'wc') {
            conditions.push(comment);
          }
        }
        return { repeats, conditions };
      };
      const visible = comments();
      const visibleLabels = root.querySelectorAll('.active-label').length;
      host.options = host.options.map((option: unknown) => ({
        ...(option as object),
        active: false,
      }));
      await Promise.resolve();
      const hidden = comments();
      host.clearOptions();
      await Promise.resolve();
      const cleared = comments();
      return {
        visibleLabels,
        visibleRepeats: visible.repeats,
        visibleConditions: visible.conditions.length,
        hiddenConditions: hidden.conditions.length,
        retainedConditions:
          hidden.conditions.filter((anchor) => anchor.isConnected).length,
        clearedRepeats: cleared.repeats,
        clearedConditions: cleared.conditions.length,
      };
    });

    expect(result).toEqual({
      visibleLabels: 128,
      visibleRepeats: 3,
      visibleConditions: 0,
      hiddenConditions: 384,
      retainedConditions: 0,
      clearedRepeats: 3,
      clearedConditions: 0,
    });
  });
});
