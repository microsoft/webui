// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

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
    const adjacentText = await page.locator('test-text-only-repeat .adjacent-repeats').evaluate(
      (element) => {
        let text = '';
        const nodes = element.childNodes;
        for (let i = 0; i < nodes.length; i++) {
          const node = nodes[i];
          if (node.nodeType === Node.TEXT_NODE) text += node.textContent?.trim() ?? '';
        }
        return text;
      },
    );
    expect(adjacentText).toBe('Relevance');
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
    const adjacentText = await page.locator('test-text-only-repeat .adjacent-repeats').evaluate(
      (element) => {
        let text = '';
        const nodes = element.childNodes;
        for (let i = 0; i < nodes.length; i++) {
          const node = nodes[i];
          if (node.nodeType === Node.TEXT_NODE) text += node.textContent?.trim() ?? '';
        }
        return text;
      },
    );
    expect(adjacentText).toBe('Trending');
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

  test('removes hidden conditional anchors with adjacent repeat items', async ({ page }) => {
    const result = await page.evaluate(async () => {
      const host = document.querySelector('test-text-only-repeat') as any;
      host.onUpdate();
      await Promise.resolve();
      const root = host.shadowRoot ?? host;
      const hiddenAnchors: Comment[] = [];
      const walker = document.createTreeWalker(root, NodeFilter.SHOW_COMMENT);
      let node: Node | null;
      while ((node = walker.nextNode())) {
        const comment = node as Comment;
        if (comment.data === '' || comment.data === 'wc') {
          hiddenAnchors.push(comment);
        }
      }

      host.clearOptions();
      await Promise.resolve();
      let repeatAnchors = 0;
      const remaining = document.createTreeWalker(root, NodeFilter.SHOW_COMMENT);
      while ((node = remaining.nextNode())) {
        if ((node as Comment).data === 'wr') repeatAnchors++;
      }

      return {
        hiddenAnchors: hiddenAnchors.length,
        retainedAnchors: hiddenAnchors.filter((anchor) => anchor.isConnected).length,
        repeatAnchors,
      };
    });

    expect(result.hiddenAnchors).toBeGreaterThan(0);
    expect(result.retainedAnchors).toBe(0);
    expect(result.repeatAnchors).toBe(3);
  });

  test('retains only repeat anchors when all conditions are visible at scale', async ({ page }) => {
    const result = await page.evaluate(async () => {
      const host = document.querySelector('test-text-only-repeat') as any;
      host.options = Array.from({ length: 128 }, (_, index) => ({
        title: `Option ${index}`,
        active: true,
      }));
      await Promise.resolve();
      const root = host.shadowRoot ?? host;
      let repeatAnchors = 0;
      let conditionalAnchors = 0;
      let totalComments = 0;
      const walker = document.createTreeWalker(root, NodeFilter.SHOW_COMMENT);
      let node: Node | null;
      while ((node = walker.nextNode())) {
        totalComments++;
        const data = (node as Comment).data;
        if (data === 'wr') repeatAnchors++;
        else if (data === '' || data === 'wc') conditionalAnchors++;
      }
      return {
        activeLabels: root.querySelectorAll('.active-label').length,
        adjacentLabels: root.querySelectorAll('.adjacent-label').length,
        repeatAnchors,
        conditionalAnchors,
        totalComments,
      };
    });

    expect(result).toEqual({
      activeLabels: 128,
      adjacentLabels: 128,
      repeatAnchors: 3,
      conditionalAnchors: 0,
      totalComments: 3,
    });
  });
});
