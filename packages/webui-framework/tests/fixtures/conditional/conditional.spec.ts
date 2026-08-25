// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('conditional fixture', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/conditional/fixture.html');
    await page.waitForSelector('test-conditional');
    await page.waitForFunction(() => {
      const conditional = document.querySelector('test-conditional');
      const ranges = document.querySelector('test-conditional-hydration-ranges');
      const interleaved = document.querySelector('test-conditional-interleaved');
      const escape = document.querySelector('test-conditional-block-escape');
      return conditional && (conditional as any).$ready === true
        && ranges && (ranges as any).$ready === true
        && interleaved && (interleaved as any).$ready === true
        && escape && (escape as any).$ready === true;
    });
  });

  test('retains anchors only while conditional bodies are absent', async ({ page }) => {
    const result = await page.evaluate(async () => {
      const host = document.querySelector('test-conditional') as any;
      const root = host.shadowRoot ?? host;
      const button = root.querySelector('.toggle');
      const sentinel = root.querySelector('.empty-sentinel');
      const nextMeaningful = (start: ChildNode | null): ChildNode | null => {
        let node = start?.nextSibling ?? null;
        while (
          node?.nodeType === Node.TEXT_NODE
          && node.textContent?.trim() === ''
        ) {
          node = node.nextSibling;
        }
        return node;
      };
      const previousMeaningful = (start: ChildNode | null): ChildNode | null => {
        let node = start?.previousSibling ?? null;
        while (
          node?.nodeType === Node.TEXT_NODE
          && node.textContent?.trim() === ''
        ) {
          node = node.previousSibling;
        }
        return node;
      };
      const isComment = (node: ChildNode | null, data: string): boolean =>
        node?.nodeType === Node.COMMENT_NODE
        && (node as Comment).data === data;

      const initialDetails = root.querySelector('.details');
      const initialVisibleAnchor = isComment(
        previousMeaningful(initialDetails),
        'wc',
      );
      host.open = false;
      await Promise.resolve();
      const firstAnchor = nextMeaningful(button);
      host.open = true;
      await Promise.resolve();
      const firstRemoved = firstAnchor?.isConnected === false;
      const restoredDetails = root.querySelector('.details');
      const restoredVisibleAnchor = isComment(
        previousMeaningful(restoredDetails),
        '',
      );
      host.open = false;
      await Promise.resolve();
      const secondAnchor = nextMeaningful(button);

      const anchor = previousMeaningful(sentinel);
      host.empty = true;
      await Promise.resolve();
      const sameWhileVisible = previousMeaningful(sentinel) === anchor;
      host.empty = false;
      await Promise.resolve();

      return {
        initialVisibleAnchor,
        firstIsAnchor: isComment(firstAnchor, ''),
        firstRemoved,
        restoredVisibleAnchor,
        secondIsAnchor: isComment(secondAnchor, ''),
        replaced: firstAnchor !== secondAnchor,
        emptyIsAnchor: isComment(anchor, 'wc'),
        emptyReused: sameWhileVisible
          && previousMeaningful(sentinel) === anchor
          && anchor?.isConnected === true,
      };
    });

    expect(result).toEqual({
      initialVisibleAnchor: false,
      firstIsAnchor: true,
      firstRemoved: true,
      restoredVisibleAnchor: false,
      secondIsAnchor: true,
      replaced: true,
      emptyIsAnchor: true,
      emptyReused: true,
    });
  });

  test('toggles the conditional body on click', async ({ page }) => {
    await page.locator('test-conditional .toggle').click();
    await expect(page.locator('test-conditional .details')).toHaveCount(0);

    await page.locator('test-conditional .toggle').click();
    await expect(page.locator('test-conditional .details')).toHaveText('Details');
  });

  test('toggles the client-created conditional body on click', async ({ page }) => {
    await expect(page.locator('test-conditional-client .details')).toHaveText('Details');

    await page.locator('test-conditional-client .toggle').click();
    await expect(page.locator('test-conditional-client .details')).toHaveCount(0);

    await page.locator('test-conditional-client .toggle').click();
    await expect(page.locator('test-conditional-client .details')).toHaveText('Details');
  });

  test('keeps a static sibling outside an empty SSR conditional', async ({ page }) => {
    const host = page.locator('test-conditional-hydration-ranges');
    await expect(host.locator('.mismatch-details')).toHaveCount(0);
    await expect(host.locator('.static-sibling')).toHaveText('Static sibling');

    await host.locator('.mismatch-toggle').click();
    await expect(host.locator('.mismatch-details')).toHaveCount(0);
    await expect(host.locator('.static-sibling')).toHaveText('Static sibling');

    await host.locator('.mismatch-toggle').click();
    await expect(host.locator('.mismatch-details')).toHaveText('Client-only details');
    await expect(host.locator('.static-sibling')).toHaveText('Static sibling');
  });

  test('hydrates nested marker ranges without stale or duplicated roots', async ({ page }) => {
    const host = page.locator('test-conditional-hydration-ranges');
    await expect(host.locator('.nested-details')).toHaveCount(0);
    await expect(host.locator('.outer-details')).toHaveText('Outer details');

    await host.locator('.outer-toggle').click();
    await expect(host.locator('.outer-details')).toHaveCount(0);
    await expect(host.locator('.static-sibling')).toHaveText('Static sibling');

    await host.locator('.outer-toggle').click();
    await expect(host.locator('.outer-details')).toHaveCount(1);
    await expect(host.locator('.nested-details')).toHaveCount(0);
  });

  test('compacts a resnapshotted root repeat when its condition hides', async ({ page }) => {
    const result = await page.evaluate(async () => {
      const host = document.querySelector('test-conditional-hydration-ranges') as any;
      const root = host.shadowRoot ?? host;
      host.snapshotItems = ['a', 'b', 'c', 'd'];
      host.snapshotOpen = true;
      await Promise.resolve();
      host.snapshotItems = ['a', 'b', 'c', 'd', 'e'];
      await Promise.resolve();

      const fifth = root.querySelectorAll('.snapshot-item')[4];
      const rootInstance = host.$root as { nodes: Node[] };
      rootInstance.nodes = Array.from(root.childNodes);
      host.snapshotOpen = false;
      await Promise.resolve();

      return {
        rendered: root.querySelectorAll('.snapshot-item').length,
        connected: fifth?.isConnected ?? true,
        retained: rootInstance.nodes.includes(fifth),
      };
    });

    expect(result).toEqual({
      rendered: 0,
      connected: false,
      retained: false,
    });
  });

  test('toggles boolean attributes reactively', async ({ page }) => {
    await page.evaluate(() => {
      const host = document.querySelector('test-conditional') as { busy: boolean } | null;
      if (host) {
        host.busy = true;
      }
    });

    await expect(page.locator('test-conditional .toggle')).toBeDisabled();

    await page.evaluate(() => {
      const host = document.querySelector('test-conditional') as { busy: boolean } | null;
      if (host) {
        host.busy = false;
      }
    });

    await expect(page.locator('test-conditional .toggle')).toBeEnabled();
  });

  test('preserves text state when the same path also drives a conditional', async ({ page }) => {
    await page.evaluate(() => {
      const host = document.querySelector('test-conditional') as { busy: boolean } | null;
      if (host) {
        host.busy = true;
      }
    });

    await expect(page.locator('test-conditional .details')).toHaveText('Details');
    await expect(page.locator('test-conditional .toggle')).toBeDisabled();
  });

  test('negation simulates else branch — shows alternate when condition is false', async ({ page }) => {
    // !open is hidden when open=true
    await expect(page.locator('test-conditional .negated')).toHaveCount(0);

    await page.locator('test-conditional .toggle').click();
    // Now open=false, !open shows, details hides
    await expect(page.locator('test-conditional .details')).toHaveCount(0);
    await expect(page.locator('test-conditional .negated')).toHaveText('Negated visible');

    await page.locator('test-conditional .toggle').click();
    await expect(page.locator('test-conditional .details')).toHaveText('Details');
    await expect(page.locator('test-conditional .negated')).toHaveCount(0);
  });

  test('compound && condition requires both operands', async ({ page }) => {
    await expect(page.locator('test-conditional .compound-and')).toHaveText('And visible');

    await page.evaluate(() => {
      (document.querySelector('test-conditional') as any).busy = true;
    });
    await expect(page.locator('test-conditional .compound-and')).toHaveCount(0);
  });

  test('compound || condition requires at least one operand', async ({ page }) => {
    await expect(page.locator('test-conditional .compound-or')).toHaveText('Or visible');

    await page.locator('test-conditional .toggle').click();
    // open=false, busy=false → both false → hidden
    await expect(page.locator('test-conditional .compound-or')).toHaveCount(0);
  });

  test('comparison operator > evaluates numeric values', async ({ page }) => {
    await expect(page.locator('test-conditional .gt-zero')).toHaveText('Positive');

    await page.evaluate(() => {
      (document.querySelector('test-conditional') as any).count = 0;
    });
    await expect(page.locator('test-conditional .gt-zero')).toHaveCount(0);
  });

  test('keeps a nested block hydration inside its own marker range', async ({ page }) => {
    const host = page.locator('test-conditional-block-escape');

    // SSR renders every branch; hydration must adopt them, not re-create them.
    await expect(host.locator('.wrap .inner-hit')).toHaveText('inner');
    await expect(host.locator('.inner-hit')).toHaveCount(1);
    await expect(host.locator('.after-hit')).toHaveCount(1);

    // The inner conditional lives inside the outer block, so hydrating that
    // block must not reach the sibling conditional that follows it.
    await page.evaluate(() => {
      (document.querySelector('test-conditional-block-escape') as any).escInner = false;
    });
    await expect(host.locator('.inner-hit')).toHaveCount(0);
    await expect(host.locator('.after-hit')).toHaveCount(1);

    await page.evaluate(() => {
      (document.querySelector('test-conditional-block-escape') as any).escInner = true;
    });
    await expect(host.locator('.wrap .inner-hit')).toHaveText('inner');
    await expect(host.locator('.inner-hit')).toHaveCount(1);
  });

  test('removes nested anchors with their enclosing conditional', async ({ page }) => {
    const result = await page.evaluate(async () => {
      const host = document.querySelector('test-conditional-block-escape') as any;
      const root = host.shadowRoot ?? host;
      const findInnerAnchor = (): Comment | null => {
        const wrap = root.querySelector('.wrap');
        if (!wrap) return null;
        const walker = document.createTreeWalker(wrap, NodeFilter.SHOW_COMMENT);
        let node: Node | null;
        while ((node = walker.nextNode())) {
          if ((node as Comment).data === '') return node as Comment;
        }
        return null;
      };

      host.escInner = false;
      await Promise.resolve();
      const firstAnchor = findInnerAnchor();
      host.escInner = true;
      await Promise.resolve();
      const firstRemoved = firstAnchor?.isConnected === false;
      host.escInner = false;
      await Promise.resolve();
      const secondAnchor = findInnerAnchor();
      host.escOuter = false;
      await Promise.resolve();

      return {
        firstCreated: firstAnchor !== null,
        firstRemoved,
        secondCreated: secondAnchor !== null,
        secondReplaced: secondAnchor !== firstAnchor,
        secondRemovedWithOuter: secondAnchor?.isConnected === false,
      };
    });

    expect(result).toEqual({
      firstCreated: true,
      firstRemoved: true,
      secondCreated: true,
      secondReplaced: true,
      secondRemovedWithOuter: true,
    });
  });

  test('anchors a sibling conditional after a root-level nested conditional', async ({ page }) => {
    const host = page.locator('test-conditional-sibling-after-nested');

    // SSR: outer branch is open, inner is closed, tail is open.
    await expect(host.locator('.nested-outer')).toHaveText('outer');
    await expect(host.locator('.nested-inner')).toHaveCount(0);
    await expect(host.locator('.nested-tail')).toHaveText('tail');
    await expect(host.locator('.nested-static')).toHaveText('static');

    // The tail conditional must own its own marker, not the inner one.
    await page.evaluate(() => {
      (document.querySelector('test-conditional-sibling-after-nested') as any).tail = false;
    });
    await expect(host.locator('.nested-tail')).toHaveCount(0);
    await expect(host.locator('.nested-outer')).toHaveText('outer');
    await expect(host.locator('.nested-static')).toHaveText('static');

    await page.evaluate(() => {
      (document.querySelector('test-conditional-sibling-after-nested') as any).tail = true;
    });
    await expect(host.locator('.nested-tail')).toHaveCount(1);
    await expect(host.locator('.nested-tail')).toHaveText('tail');

    // The inner conditional still belongs to the outer branch.
    await page.evaluate(() => {
      (document.querySelector('test-conditional-sibling-after-nested') as any).inner = true;
    });
    await expect(host.locator('.nested-inner')).toHaveCount(1);
    await expect(host.locator('.nested-inner')).toHaveText('inner');
    await expect(host.locator('.nested-tail')).toHaveCount(1);
  });

  test('anchors a conditional that revisits an earlier parent to its own marker', async ({ page }) => {
    const host = page.locator('test-conditional-interleaved');

    // SSR places every branch correctly.
    await expect(host.locator('.head')).toHaveText('head');
    await expect(host.locator('.stats .cell .num.pending')).toHaveText('pending');
    await expect(host.locator('.head .num')).toHaveCount(0);

    // A conditional nested inside the second root-level `<if>` updates on the client.
    await page.evaluate(() => {
      (document.querySelector('test-conditional-interleaved') as any).value = '42';
    });

    await expect(host.locator('.stats .cell .num')).toHaveText('42');
    await expect(host.locator('.stats .cell .num.pending')).toHaveCount(0);
    await expect(host.locator('.head .num')).toHaveCount(0);
    await expect(host.locator('.head')).toHaveText('head');
  });

  test('toggles a root conditional that revisits an earlier parent', async ({ page }) => {
    const host = page.locator('test-conditional-interleaved');

    await page.evaluate(() => {
      (document.querySelector('test-conditional-interleaved') as any).showStats = false;
    });
    await expect(host.locator('.head')).toHaveCount(0);
    await expect(host.locator('.stats')).toHaveCount(0);
    await expect(host.locator('.box .box-body')).toHaveText('body');
    await expect(host.locator('.box .full-title')).toHaveText('full title');

    await page.evaluate(() => {
      (document.querySelector('test-conditional-interleaved') as any).showStats = true;
    });
    await expect(host.locator('.head')).toHaveText('head');
    await expect(host.locator('.stats .cell .num.pending')).toHaveText('pending');
    await expect(host.locator('.box .full-title')).toHaveText('full title');
  });
});
