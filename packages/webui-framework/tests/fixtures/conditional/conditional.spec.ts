// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';
import { writeFile } from 'node:fs/promises';
import type { TestConditionTruthiness } from './element.js';
import { hydrationRemovals, recordHydrationRemovals } from '../../hydration-removals.js';

test('hydrates multi-node conditional ranges in place', async ({ page }) => {
  await recordHydrationRemovals(page, 'test-conditional-hydration-ranges');
  await page.goto('/conditional/fixture.html');
  expect(await hydrationRemovals(page, 'test-conditional-hydration-ranges')).toEqual([]);
});

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

  test('removes static text beside a conditional element with the element', async ({ page }) => {
    const host = page.locator('test-conditional-hydration-ranges');
    const labelText = () => host.evaluate(element =>
      element.shadowRoot!.textContent!.split('after label').length - 1);
    expect(await labelText()).toBe(1);

    await host.locator('.outer-toggle').click();
    await expect(host.locator('.outer-label')).toHaveCount(0);
    expect(await labelText()).toBe(0);

    await host.locator('.outer-toggle').click();
    await expect(host.locator('.outer-label')).toHaveText('Outer label');
    expect(await labelText()).toBe(1);
  });

  test('keeps inline separators at conditional edges when recreated', async ({ page }) => {
    const host = page.locator('test-conditional-hydration-ranges');
    const edges = host.locator('.inline-edges');
    await expect(edges).toHaveJSProperty('textContent', 'Z A B');

    await host.locator('.outer-toggle').click();
    await expect(edges).toHaveJSProperty('textContent', 'ZB');

    await host.locator('.outer-toggle').click();
    await expect(edges).toHaveJSProperty('textContent', 'Z A B');
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

test('keeps bare collection conditions in parity across SSR and client updates', async ({
  page, browser,
}, testInfo) => {
  const tag = 'test-condition-truthiness';
  const cases = [
    'items', 'trailing', 'record', 'items-length', 'trailing-length', 'record-field',
    'empty-items', 'trailing-and', 'record-or',
    'false', 'zero', 'empty-text', 'null', 'missing', 'true',
  ];
  type ReadyElement = HTMLElement & { $ready?: boolean };
  const read = () => page.evaluate(({ tag, cases }) =>
    Array.from(document.querySelectorAll<TestConditionTruthiness>(tag), (host) => {
      const root = host.shadowRoot;
      if (!root) throw new Error(`Missing shadow root for ${host.id}`);
      return {
        id: host.id,
        counts: Object.fromEntries(cases.map((name) => [
          name, root.querySelectorAll(`[data-case="${name}"]`).length,
        ])),
        html: root.innerHTML,
        checked: root.querySelector<HTMLInputElement>('.collection-flag')?.checked,
        state: { items: host.items, actions: host.actions, record: host.record },
      };
    }), { tag, cases });
  const snapshots: Record<string, Awaited<ReturnType<typeof read>>> = {};
  const diagnostics: { kind: string; message: string }[] = [];
  let hydrationBranchNodesPreserved: boolean | undefined;
  const check = (
    host: Awaited<ReturnType<typeof read>>[number],
    populated: boolean,
    phase: string,
    hydrated = true,
  ): void => {
    // Keep the desired server semantics; empty collections must stay absent.
    expect.soft(host.counts, `${host.id}: ${phase}`).toEqual({
      items: Number(populated),
      trailing: Number(populated),
      record: Number(populated),
      'items-length': Number(populated),
      'trailing-length': Number(populated),
      'record-field': Number(populated),
      'empty-items': Number(!populated),
      'trailing-and': Number(populated),
      'record-or': Number(populated),
      false: 0,
      zero: 0,
      'empty-text': 0,
      null: 0,
      missing: 0,
      true: 1,
    });
    expect.soft(host.checked, `${host.id}: ${phase} boolean attribute`).toBe(populated);
    if (hydrated) {
      expect(host.state).toEqual({
        items: populated ? ['Item'] : [],
        actions: { trailing: populated ? ['Action'] : [] },
        record: populated ? { label: 'Record' } : {},
      });
    }
  };
  page.on('console', (message) => {
    if (message.type() === 'error' || message.type() === 'warning') {
      diagnostics.push({ kind: message.type(), message: message.text() });
    }
  });
  page.on('pageerror', (error) => {
    diagnostics.push({ kind: 'pageerror', message: error.message });
  });

  // Pause the fixture bundle to inspect the real SSR DOM before any upgrade.
  const scriptGate = Promise.withResolvers<void>();
  await page.route('**/dist/conditional/element.js', async (route) => {
    await scriptGate.promise;
    await route.continue();
  });
  try {
    const response = await page.goto('/conditional/fixture.html', { waitUntil: 'commit' });
    if (!response) throw new Error('Fixture navigation returned no response');
    expect(response.ok()).toBe(true);
    const htmlPath = testInfo.outputPath('server-rendered.html');
    await writeFile(htmlPath, await response.body());
    await testInfo.attach('server-rendered.html', { path: htmlPath, contentType: 'text/html' });
    await page.waitForFunction((tag) => {
      const hosts = document.querySelectorAll(tag);
      return hosts.length === 2 && Array.from(hosts).every((host) => host.shadowRoot);
    }, tag);
    snapshots.ssr = await read();
    for (const host of snapshots.ssr) check(host, host.id === 'populated', 'SSR', false);
    const nodes = await page.evaluateHandle((tag) =>
      Array.from(document.querySelectorAll(tag), (host) => ({
        id: host.id,
        nodes: Array.from(host.shadowRoot!.querySelectorAll('[data-case]')),
      })), tag);

    scriptGate.resolve();
    await page.waitForLoadState('load');
    await page.waitForFunction((tag) =>
      Array.from(document.querySelectorAll<ReadyElement>(tag))
        .every((host) => host.$ready), tag);
    snapshots.hydrated = await read();
    for (const host of snapshots.hydrated) check(host, host.id === 'populated', 'hydration');
    hydrationBranchNodesPreserved = await nodes.evaluate((hosts) =>
      hosts.every(({ id, nodes }) => {
        const current = document.getElementById(id)?.shadowRoot?.querySelectorAll('[data-case]');
        return current?.length === nodes.length
          && nodes.every((node, i) => node === current[i]);
      }));
    expect(hydrationBranchNodesPreserved, 'hydration retains SSR branch nodes').toBe(true);
    await nodes.dispose();

    for (const [step, action] of ['clear', 'populate', 'clear'].entries()) {
      for (const id of ['empty', 'populated']) {
        await page.locator(`#${id} .${action}`).click();
        await expect(page.locator(`#${id} .phase`)).toHaveText(
          action === 'populate' ? 'populated' : 'empty',
        );
      }
      const phase = `${step}-${action}`;
      snapshots[phase] = await read();
      for (const host of snapshots[phase]) check(host, action === 'populate', phase);
    }

    await page.evaluate((tag) => {
      for (const populated of [false, true]) {
        const host = document.createElement(tag) as TestConditionTruthiness;
        host.id = populated ? 'client-populated' : 'client-empty';
        if (populated) host.populate();
        document.body.append(host);
      }
    }, tag);
    await page.waitForFunction(() =>
      ['client-empty', 'client-populated'].every((id) =>
        (document.getElementById(id) as ReadyElement | null)?.$ready));
    snapshots.created = await read();
    for (const host of snapshots.created) {
      if (host.id.startsWith('client-')) {
        check(host, host.id === 'client-populated', 'client creation');
      }
    }
    const screenshotPath = testInfo.outputPath('truthiness-final.png');
    await page.screenshot({ path: screenshotPath, fullPage: true });
    await testInfo.attach('truthiness-final.png', { path: screenshotPath, contentType: 'image/png' });

    // The existing range-ownership case deliberately disagrees with SSR.
    expect.soft(diagnostics, 'only the deliberate clientOnlyOpen mismatch is expected').toEqual([
      {
        kind: 'warning',
        message: expect.stringContaining(
          'Hydration mismatch on <test-conditional-hydration-ranges>: "clientOnlyOpen" changed',
        ),
      },
    ]);
  } finally {
    scriptGate.resolve();
    const path = testInfo.outputPath('truthiness-evidence.json');
    await writeFile(path, JSON.stringify({
      browser: browser.version(), diagnostics, snapshots, hydrationBranchNodesPreserved,
    }, null, 2));
    await testInfo.attach('truthiness-evidence.json', { path, contentType: 'application/json' });
  }
});
