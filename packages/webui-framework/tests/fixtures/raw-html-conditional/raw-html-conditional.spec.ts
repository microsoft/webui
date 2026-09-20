// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('raw HTML inside conditional', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/raw-html-conditional/fixture.html');
    await page.waitForSelector('test-raw-html');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-raw-html');
      return el && (el as any).$ready === true;
    });
  });

  test('header retains text after hydration when sibling has {{{raw}}}', async ({ page }) => {
    // After SSR + hydration, the structured header must still contain "Alice",
    // not the raw HTML body content.
    await expect(page.locator('test-raw-html .header .name')).toHaveText('Alice');
  });

  test('reactive raw HTML updates preserve and order sibling ranges', async ({ page }) => {
    const body = page.locator('test-raw-html .body');
    await expect(body.locator(':scope > .raw-initial')).toHaveText('Hello');

    await page.evaluate(() => {
      const host = document.querySelector('test-raw-html') as HTMLElement & {
        rawHtml: string;
      };
      host.rawHtml = '<b class="raw-first">One</b><!--/wh--><i class="raw-second">Two</i>';
    });

    await expect(body.locator(':scope > .before')).toHaveText('Before');
    await expect(body.locator(':scope > .raw-first')).toHaveText('One');
    await expect(body.locator(':scope > .raw-second')).toHaveText('Two');
    await expect(body.locator(':scope > .conditional')).toHaveText('Conditional');
    await expect(body.locator(':scope > .after')).toHaveText('After');
    await expect(body.locator(':scope > *')).toHaveClass([
      'before',
      'raw-first',
      'raw-second',
      'conditional',
      'after',
    ]);

    await page.evaluate(() => {
      const host = document.querySelector('test-raw-html') as HTMLElement & {
        rawHtml: string;
      };
      host.rawHtml = '';
    });
    await expect(body.locator(':scope > *')).toHaveClass([
      'before',
      'conditional',
      'after',
    ]);

    await page.evaluate(() => {
      const host = document.querySelector('test-raw-html') as HTMLElement & {
        rawHtml: string;
      };
      host.rawHtml = '<mark class="raw-only">Only</mark>';
    });
    await expect(body.locator(':scope > *')).toHaveClass([
      'before',
      'raw-only',
      'conditional',
      'after',
    ]);

    await page.evaluate(() => {
      const host = document.querySelector('test-raw-html') as HTMLElement & {
        rawHtml: string;
      };
      host.rawHtml = '<b class="raw-first">One</b><i class="raw-second">Two</i>';
    });
    await expect(body.locator(':scope > *')).toHaveClass([
      'before',
      'raw-first',
      'raw-second',
      'conditional',
      'after',
    ]);

    await page.evaluate(() => {
      const host = document.querySelector('test-raw-html') as HTMLElement & {
        expanded: boolean;
      };
      host.expanded = false;
    });
    await expect(page.locator('test-raw-html .body')).toHaveCount(0);

    await page.evaluate(() => {
      const host = document.querySelector('test-raw-html') as HTMLElement & {
        expanded: boolean;
      };
      host.expanded = true;
    });
    await expect(body.locator(':scope > *')).toHaveClass([
      'before',
      'raw-first',
      'raw-second',
      'conditional',
      'after',
    ]);

    await page.evaluate(async () => {
      const host = document.querySelector('test-raw-html') as HTMLElement & {
        rawHtml: string;
      };
      host.remove();
      await Promise.resolve();
      host.rawHtml = '<u class="raw-reconnected">Reconnected</u>';
      document.body.appendChild(host);
    });
    await expect(body.locator(':scope > *')).toHaveClass([
      'before',
      'raw-reconnected',
      'conditional',
      'after',
    ]);
  });

  test('parses reactive raw table rows in the tbody context', async ({ page }) => {
    const rows = page.locator('test-raw-html .rows');
    await expect(rows.locator(':scope > tr')).toHaveClass([
      'before-row',
      'raw-row raw-row-initial',
      'conditional-row',
      'after-row',
    ]);

    await page.evaluate(() => {
      const host = document.querySelector('test-raw-html') as HTMLElement & {
        rowsHtml: string;
      };
      host.rowsHtml = [
        '<tr class="raw-row raw-row-first"><td>First</td></tr>',
        '<tr class="raw-row raw-row-second"><td>Second</td></tr>',
      ].join('');
    });

    await expect(rows.locator(':scope > tr')).toHaveClass([
      'before-row',
      'raw-row raw-row-first',
      'raw-row raw-row-second',
      'conditional-row',
      'after-row',
    ]);
    await expect(rows.locator(':scope > .raw-row > td')).toHaveText(['First', 'Second']);
    expect(await rows.locator(':scope > .raw-row').evaluateAll((elements) => (
      elements.every((element) => (
        element instanceof HTMLTableRowElement
        && element.firstElementChild instanceof HTMLTableCellElement
      ))
    ))).toBe(true);
  });

  test('header does not contain raw HTML body content', async ({ page }) => {
    const headerHtml = await page.locator('test-raw-html .header').innerHTML();
    expect(headerHtml).not.toContain('Hello');
  });

  test('escaped text bindings stay aligned around an inline raw range', async ({ page }) => {
    const line = page.locator('test-raw-html .inline-text');
    await expect(line).toHaveText('Before raw After Alice');
    await expect(line.locator(':scope > .inline-raw')).toHaveText('raw');

    await page.evaluate(() => {
      const host = document.querySelector('test-raw-html') as HTMLElement & {
        inlineName: string;
        inlineRawHtml: string;
      };
      host.inlineName = 'Bob';
      host.inlineRawHtml = '<i class="inline-raw">updated</i>';
    });

    await expect(line).toHaveText('Before updated After Bob');
    await expect(line.locator(':scope > .inline-raw')).toHaveText('updated');
  });

  test('updates adjacent raw ranges independently', async ({ page }) => {
    const container = page.locator('test-raw-html .adjacent-raw');
    await expect(container.locator(':scope > *')).toHaveClass([
      'adjacent-first',
      'adjacent-second',
    ]);

    await page.evaluate(() => {
      const host = document.querySelector('test-raw-html') as HTMLElement & {
        firstRawHtml: string;
      };
      host.firstRawHtml =
        '<strong class="adjacent-first-a">First A</strong>'
        + '<em class="adjacent-first-b">First B</em>';
    });
    await expect(container.locator(':scope > *')).toHaveClass([
      'adjacent-first-a',
      'adjacent-first-b',
      'adjacent-second',
    ]);

    await page.evaluate(() => {
      const host = document.querySelector('test-raw-html') as HTMLElement & {
        secondRawHtml: string;
      };
      host.secondRawHtml = '';
    });
    await expect(container.locator(':scope > *')).toHaveClass([
      'adjacent-first-a',
      'adjacent-first-b',
    ]);
  });

  test('hydrates raw CSS signals as marker-free text', async ({ page }) => {
    const target = page.locator('test-raw-html .raw-context-target');
    await expect(target).toHaveCSS('color', 'rgb(1, 2, 3)');

    await page.evaluate(() => {
      const host = document.querySelector('test-raw-html') as HTMLElement & {
        styleRule: string;
      };
      host.styleRule = 'color: rgb(4, 5, 6);';
    });
    await expect(target).toHaveCSS('color', 'rgb(4, 5, 6)');

    const markerText = await page.locator('test-raw-html style').textContent();
    expect(markerText).not.toContain('<!--w');
  });

  test('client-created raw HTML uses the same sibling structure', async ({ page }) => {
    await page.evaluate(() => {
      const host = document.createElement('test-raw-html') as HTMLElement & {
        expanded: boolean;
        name: string;
        rawHtml: string;
      };
      host.id = 'client-raw-html';
      document.body.appendChild(host);
      host.name = 'Client';
      host.rawHtml = '<b class="raw-first">One</b><i class="raw-second">Two</i>';
    });

    const body = page.locator('#client-raw-html .body');
    await expect(body.locator(':scope > *')).toHaveClass([
      'before',
      'raw-first',
      'raw-second',
      'conditional',
      'after',
    ]);

    await page.evaluate(() => {
      const host = document.querySelector('#client-raw-html') as HTMLElement & {
        expanded: boolean;
      };
      host.expanded = false;
    });
    await expect(page.locator('#client-raw-html .body')).toHaveCount(0);
  });

  test('keyed repeat moves raw ranges by identity and removes them without ghosts', async ({
    page,
  }) => {
    await page.waitForFunction(() => {
      const element = document.querySelector('test-raw-html-keyed-repeat');
      return element && (element as unknown as { $ready?: boolean }).$ready === true;
    });

    const host = page.locator('test-raw-html-keyed-repeat');
    const items = host.locator('.keyed-raw-item');
    await expect(items).toHaveCount(3);

    await page.evaluate(() => {
      const host = document.querySelector('test-raw-html-keyed-repeat') as HTMLElement & {
        updateItemRaw(id: string): void;
      };
      host.updateItemRaw('b');
    });
    await expect(host.locator('[data-owner="b"]')).toHaveText([
      'b updated 1',
      'b updated 2',
    ]);

    await page.evaluate(() => {
      const host = document.querySelector('test-raw-html-keyed-repeat') as HTMLElement & {
        updateItemRawHtml(id: string, rawHtml: string): void;
      };
      host.updateItemRawHtml(
        'b',
        '<strong class="raw-node" data-owner="b">B updated 1</strong>'
          + '<em class="raw-node" data-owner="b">B updated 2</em>',
      );
    });
    await expect(host.locator('[data-owner="b"]')).toHaveText(['B updated 1', 'B updated 2']);

    await page.evaluate(() => {
      const host = document.querySelector('test-raw-html-keyed-repeat') as HTMLElement & {
        reverseItems(): void;
      };
      const root = host.shadowRoot ?? host;
      const snapshots = new Map<string, Node[]>();
      root.querySelectorAll('.keyed-raw-item').forEach((item) => {
        const owned: Node[] = [];
        let node = item.nextSibling;
        while (node) {
          const data = node.nodeType === Node.COMMENT_NODE ? (node as Comment).data : '';
          const markerDigit = data.charCodeAt(1);
          if (data.charCodeAt(0) === 119 && markerDigit >= 48 && markerDigit <= 57) {
            owned.push(node);
            const endMarker = `/${data}`;
            node = node.nextSibling;
            while (node) {
              owned.push(node);
              if (node.nodeType === Node.COMMENT_NODE
                && (node as Comment).data === endMarker) break;
              node = node.nextSibling;
            }
            break;
          }
          node = node.nextSibling;
        }
        snapshots.set(item.getAttribute('data-id') ?? '', [item, ...owned]);
      });
      (window as unknown as { __keyedRawSnapshots?: Map<string, Node[]> })
        .__keyedRawSnapshots = snapshots;
      host.reverseItems();
    });

    const reorderedIds = await items.evaluateAll((elements) => (
      elements.map((element) => element.getAttribute('data-id'))
    ));
    expect(reorderedIds).toEqual(['c', 'b', 'a']);

    const identityById = await page.evaluate(() => {
      const host = document.querySelector('test-raw-html-keyed-repeat');
      const root = host?.shadowRoot ?? host;
      const snapshots = (window as unknown as {
        __keyedRawSnapshots?: Map<string, Node[]>;
      }).__keyedRawSnapshots;
      if (!root || !snapshots) return [];

      return Array.from(root.querySelectorAll('.keyed-raw-item')).map((item) => {
        const id = item.getAttribute('data-id') ?? '';
        const original = snapshots.get(id);
        const owned: Node[] = [];
        let node = item.nextSibling;
        while (node) {
          const data = node.nodeType === Node.COMMENT_NODE ? (node as Comment).data : '';
          const markerDigit = data.charCodeAt(1);
          if (data.charCodeAt(0) === 119 && markerDigit >= 48 && markerDigit <= 57) {
            owned.push(node);
            const endMarker = `/${data}`;
            node = node.nextSibling;
            while (node) {
              owned.push(node);
              if (node.nodeType === Node.COMMENT_NODE
                && (node as Comment).data === endMarker) break;
              node = node.nextSibling;
            }
            break;
          }
          node = node.nextSibling;
        }
        return {
          id,
          root: original?.[0] === item,
          owned: owned.map((node, index) => original?.[index + 1] === node),
        };
      });
    });
    expect(identityById).toEqual([
      { id: 'c', root: true, owned: [true, true, true, true] },
      { id: 'b', root: true, owned: [true, true, true, true] },
      { id: 'a', root: true, owned: [true, true, true, true] },
    ]);

    await page.evaluate(() => {
      const host = document.querySelector('test-raw-html-keyed-repeat') as HTMLElement & {
        removeItem(id: string): void;
      };
      host.removeItem('b');
    });

    await expect(items).toHaveCount(2);
    expect(await items.evaluateAll((elements) => (
      elements.map((element) => element.getAttribute('data-id'))
    ))).toEqual(['c', 'a']);
    await expect(host.locator('[data-owner="b"]')).toHaveCount(0);
    await expect(host.locator('.raw-node')).toHaveCount(4);

    const markerCounts = await page.evaluate(() => {
      const host = document.querySelector('test-raw-html-keyed-repeat');
      const root = host?.shadowRoot ?? host;
      if (!root) return { starts: -1, ends: -1 };
      const walker = document.createTreeWalker(root, NodeFilter.SHOW_COMMENT);
      let starts = 0;
      let ends = 0;
      while (walker.nextNode()) {
        const data = (walker.currentNode as Comment).data;
        const startDigit = data.charCodeAt(1);
        const endDigit = data.charCodeAt(2);
        if (data.charCodeAt(0) === 119 && startDigit >= 48 && startDigit <= 57) starts += 1;
        if (data.charCodeAt(0) === 47 && data.charCodeAt(1) === 119
          && endDigit >= 48 && endDigit <= 57) ends += 1;
      }
      return { starts, ends };
    });
    expect(markerCounts).toEqual({ starts: 2, ends: 2 });
  });
});
