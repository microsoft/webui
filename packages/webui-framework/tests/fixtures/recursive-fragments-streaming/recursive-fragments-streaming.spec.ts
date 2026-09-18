// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';
import type {
  TestStreamedFragmentCapture,
  TestStreamedFragmentProps,
  TestStreamedRecursiveTree,
} from './element.js';

test('a real progressive checkpoint activates recursive calls before the response tail', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.goto('/recursive-fragments-streaming/fixture.html');
  const tree = page.locator('test-streamed-recursive-tree');
  await expect(tree.locator('.name')).toHaveText(['Root', 'Stream leaf']);
  await tree.locator('button').last().click();
  await expect(tree.locator('output')).toHaveText('Stream leaf');
  expect(await tree.evaluate((element: TestStreamedRecursiveTree) => ({
    hydrations: element.hydrations,
    tailPresent: element.tailPresentAtHydration,
  }))).toEqual({ hydrations: 1, tailPresent: false });
  await page.evaluate(() => {
    const element = document.querySelector(
      'test-streamed-recursive-tree',
    ) as TestStreamedRecursiveTree;
    element.items = [{
      id: 'root',
      name: 'Updated root',
      children: [{ id: 'next', name: 'Next leaf', children: [] }],
    }];
  });

  await expect(tree.locator('.name')).toHaveText(['Updated root', 'Next leaf']);
  await tree.locator('button').last().click();
  await expect(tree.locator('output')).toHaveText('Next leaf');
  expect(errors).toEqual([]);
});

test('streamed hydration preserves captured aliases for events and later owner updates', async ({ page, request }) => {
  const response = await request.get('/recursive-fragments-streaming/fixture.html');
  const html = await response.text();
  expect(html).toContain('"fragmentSources"');
  expect(html).toMatch(/<!--wf:\d+-->/);
  expect(html).not.toContain('"fragmentInputs"');
  await page.goto('/recursive-fragments-streaming/fixture.html');
  const capture = page.locator('test-streamed-fragment-capture');
  await expect(capture.locator('.capture')).toHaveText('Captured OLD/Resumed owner');
  await expect(capture.locator('.captured-tail')).toHaveText('Captured OLD');
  await expect(capture.locator('.owner-title')).toHaveText('Resumed owner');
  expect(await capture.evaluate((element: TestStreamedFragmentCapture) => element.source.name))
    .toBe('Replacement NEW');
  expect(await page.evaluate(() =>
    ['fragmentSources', 'fragmentSourceRefs', 'fragmentInputs'].some((key) =>
      Object.prototype.hasOwnProperty.call(window.__webui ?? {}, key),
    ),
  )).toBe(false);
  await capture.locator('.before-capture').click();
  await expect(capture.locator('.capture-selected')).toHaveText('Captured OLD/Resumed owner');
  await capture.locator('.capture').click();
  await expect(capture.locator('.capture-selected')).toHaveText('Captured OLD/Resumed owner');
  await page.evaluate(() => {
    const element = document.querySelector(
      'test-streamed-fragment-capture',
    ) as TestStreamedFragmentCapture;
    element.title = 'Client owner';
  });
  await expect(capture.locator('.owner-title')).toHaveText('Client owner');
  await expect(capture.locator('.before-capture')).toHaveText('Captured OLD/Client owner');
  await expect(capture.locator('.capture')).toHaveText('Captured OLD/Client owner');
  await capture.locator('.capture').click();
  await expect(capture.locator('.capture-selected')).toHaveText('Captured OLD/Client owner');
  expect(await capture.evaluate((element) => {
    const button = element.shadowRoot?.querySelector('.capture');
    document.body.append(element);
    return !!button && element.shadowRoot?.querySelector('.capture') === button;
  })).toBe(true);
  await expect(capture.locator('.before-capture')).toHaveText('Captured OLD/Client owner');
  await expect(capture.locator('.capture')).toHaveText('Captured OLD/Client owner');
  await page.evaluate(async () => {
    const element = document.querySelector(
      'test-streamed-fragment-capture',
    ) as TestStreamedFragmentCapture;
    element.remove();
    await new Promise<void>((resolve) => queueMicrotask(resolve));
    document.body.append(element);
  });
  await expect(capture.locator('.before-capture')).toHaveText('Captured OLD/Client owner');
  await expect(capture.locator('.capture')).toHaveText('Captured OLD/Client owner');
  await capture.locator('.capture').click();
  await expect(capture.locator('.capture-selected')).toHaveText('Captured OLD/Client owner');
  await page.evaluate(() => {
    const element = document.querySelector(
      'test-streamed-fragment-capture',
    ) as TestStreamedFragmentCapture;
    element.source = { name: 'Explicit client input' };
  });
  await expect(capture.locator('.capture')).toHaveText('Explicit client input/Client owner');
  await expect(capture.locator('.before-capture')).toHaveText('Explicit client input/Client owner');
  await expect(capture.locator('.captured-tail')).toHaveText('Explicit client input');
  await capture.locator('.capture').click();
  await expect(capture.locator('.capture-selected')).toHaveText('Explicit client input/Client owner');
});

test('first component props remain available to a suspended fragment', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.goto('/recursive-fragments-streaming/fixture.html');
  const props = page.locator('test-streamed-fragment-props');
  await expect(props.locator('.prop-capture')).toHaveText('Captured OLD/Stream forest');
  await expect(props.locator('.prop-tail')).toHaveText('Captured OLD');
  expect(await props.evaluate((element: TestStreamedFragmentProps) => element.model.name))
    .toBe('Captured OLD');
  await props.locator('.prop-capture').click();
  await expect(props.locator('.prop-selected')).toHaveText('Captured OLD/Stream forest');
  await props.evaluate((element: TestStreamedFragmentProps) => {
    element.title = 'Client prop owner';
  });
  await expect(props.locator('.prop-capture')).toHaveText('Captured OLD/Client prop owner');
  await props.evaluate((element: TestStreamedFragmentProps) => {
    element.model = { name: 'New client model' };
  });
  await expect(props.locator('.prop-capture')).toHaveText('New client model/Client prop owner');
  await props.locator('.prop-capture').click();
  await expect(props.locator('.prop-selected')).toHaveText('New client model/Client prop owner');
  expect(errors).toEqual([]);
});

test('recursive caller captures survive terminal and reconnect, then rebind and remove cleanly', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  let publishDefinitions = () => {};
  const definitionsPaused = new Promise<void>((resolve) => {
    publishDefinitions = resolve;
  });
  await page.route('**/dist/recursive-fragments-streaming/element.js', async (route) => {
    await definitionsPaused;
    await route.continue();
  });
  await page.goto('/recursive-fragments-streaming/fixture.html', { waitUntil: 'commit' });
  await expect(page.locator('footer')).toHaveText('Stream tail');
  await expect(page.locator('script[data-webui-boundary]').first()).toBeAttached();
  await expect(page.locator('webui-hydrate').first()).toBeAttached();
  const capture = page.locator('test-streamed-fragment-capture');
  const buttons = capture.locator('.tree-capture');
  await expect(capture.locator('.grove-name')).toHaveText('Captured grove OLD');
  await expect(buttons).toHaveText([
    'Captured branch OLD/Resumed owner', 'Captured leaf OLD/Resumed owner',
  ]);
  await expect(capture.locator(
    '.captured-grove > ul > li[data-id="branch"] > ul > li[data-id="leaf"] > .tree-capture',
  )).toHaveText('Captured leaf OLD/Resumed owner');
  // The compact grove template avoids whitespace-only slots synthesized during hydration.
  const ssr = await capture.locator('.captured-grove').evaluateHandle((root) => {
    const nodes: Node[] = [];
    const markers: Comment[] = [];
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_ALL);
    let node: Node | null;
    while ((node = walker.nextNode())) {
      nodes.push(node);
      if (node instanceof Comment && node.data.startsWith('wf:')) markers.push(node);
    }
    return { root, nodes, markers };
  });
  expect(await ssr.evaluate(({ markers }) => markers.map((marker) => marker.data))).toEqual([
    expect.stringMatching(/^wf:\d+$/), expect.stringMatching(/^wf:\d+$/),
  ]);
  const expectPreserved = async (): Promise<void> => {
    const identity = await ssr.evaluate(({ root, nodes, markers }) => {
      const walker = document.createTreeWalker(root, NodeFilter.SHOW_ALL);
      const current: Node[] = [];
      let node: Node | null;
      while ((node = walker.nextNode())) current.push(node);
      const describe = (node: Node | undefined) => node ? {
        type: node.nodeType,
        name: node.nodeName,
        data: node.nodeValue,
        html: node instanceof Element ? node.outerHTML.slice(0, 300) : undefined,
        connected: node.isConnected,
      } : null;
      return {
        rootConnected: root.isConnected,
        missingOriginals: nodes.filter((node) => !current.includes(node)).map(describe),
        insertions: current.filter((node) => !nodes.includes(node)).map((node) => ({
          node: describe(node),
          parent: describe(node.parentNode ?? undefined),
          previous: describe(node.previousSibling ?? undefined),
          next: describe(node.nextSibling ?? undefined),
        })),
        mismatches: nodes.flatMap((node, index) => current[index] === node ? [] : [{
          index, expected: describe(node), actual: describe(current[index]),
        }]),
        extraNodes: current.slice(nodes.length).map(describe),
        markersCleared: markers.every((marker) => marker.data === ''),
        markers: markers.map((marker) => marker.data),
      };
    });
    const values = await capture.evaluate((element: TestStreamedFragmentCapture) => ({
      owner: element.source,
      rendered: Array.from(element.shadowRoot?.querySelectorAll('.tree-capture') ?? [],
        (button) => button.textContent),
    }));
    expect(identity, JSON.stringify({ identity, values, errors }, null, 2)).toMatchObject({
      rootConnected: true, mismatches: [], extraNodes: [], markersCleared: true,
    });
  };
  const completion = await page.evaluateHandle(() => {
    const state = { complete: false };
    window.addEventListener('webui:hydration-complete', () => {
      state.complete = true;
    }, { once: true });
    return state;
  });
  publishDefinitions();
  await page.waitForLoadState('load');
  await expect.poll(() => completion.evaluate((state) => state.complete)).toBe(true);
  await expectPreserved();
  await expect(page.locator('script[data-webui-boundary], webui-hydrate')).toHaveCount(0);
  expect(await page.evaluate(() =>
    ['fragmentSources', 'fragmentSourceRefs', 'fragmentInputs'].some((key) =>
      Object.prototype.hasOwnProperty.call(window.__webui ?? {}, key),
    ),
  )).toBe(false);
  expect(await capture.evaluate((element: TestStreamedFragmentCapture) => ({
    name: element.source.name,
    leaf: element.source.branches?.[0].children?.[0].children?.[0].name,
  }))).toEqual({ name: 'Replacement NEW', leaf: 'Replacement leaf NEW' });
  await buttons.last().click();
  await expect(capture.locator('.capture-selected')).toHaveText('Captured leaf OLD/Resumed owner');
  await capture.evaluate((element: TestStreamedFragmentCapture) => {
    element.title = 'Client recursive owner';
  });
  for (const delayed of [false, true]) {
    await capture.evaluate(async (element: TestStreamedFragmentCapture, delayed) => {
      element.remove();
      if (delayed) await new Promise<void>((resolve) => queueMicrotask(resolve));
      document.body.append(element);
    }, delayed);
    await expectPreserved();
    await expect(buttons).toHaveText([
      'Captured branch OLD/Client recursive owner', 'Captured leaf OLD/Client recursive owner',
    ]);
    await buttons.last().click();
    await expect(capture.locator('.capture-selected')).toHaveText('Captured leaf OLD/Client recursive owner');
  }

  // Only the ultimate owner root is written: caller `branch` and callee `items` differ.
  await capture.evaluate((element: TestStreamedFragmentCapture) => {
    element.source = { ...element.source };
  });
  await expect(capture.locator('.grove-name')).toHaveText('Replacement grove NEW');
  await expect(buttons).toHaveText([
    'Replacement branch NEW/Client recursive owner', 'Replacement leaf NEW/Client recursive owner',
  ]);
  await expectPreserved();
  await buttons.last().click();
  await expect(capture.locator('.capture-selected')).toHaveText('Replacement leaf NEW/Client recursive owner');
  await capture.evaluate((element: TestStreamedFragmentCapture) => {
    element.showCapturedTree = false;
    element.selected = 'Removed';
  });
  await expect(buttons).toHaveCount(0);
  expect(await ssr.evaluate(({ root }) => {
    const button = root.querySelector<HTMLButtonElement>('[data-id="leaf"] > .tree-capture');
    if (!button) throw new Error('Expected the retained recursive leaf button');
    button.click();
    return !root.isConnected && !button.isConnected;
  })).toBe(true);
  await expect(capture.locator('.capture-selected')).toHaveText('Removed');
  await capture.evaluate((element: TestStreamedFragmentCapture) => {
    element.source = {
      name: 'Final input',
      branches: [{
        id: 'grove', name: 'Final grove',
        children: [{ id: 'final', name: 'Final leaf' }],
      }],
    };
    element.showCapturedTree = true;
  });
  await expect(capture.locator('.grove-name')).toHaveText('Final grove');
  await expect(buttons).toHaveText('Final leaf/Client recursive owner');
  await expect(capture.locator('[data-id="branch"], [data-id="leaf"]')).toHaveCount(0);
  await buttons.click();
  await expect(capture.locator('.capture-selected')).toHaveText('Final leaf/Client recursive owner');
  expect(errors).toEqual([]);
  await ssr.dispose();
  await completion.dispose();
});

test('rebound inputs survive teardown while later stream records are still pending', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.goto('/recursive-fragments-streaming/fixture.html?slow-stream', { waitUntil: 'commit' });
  await page.waitForFunction(() =>
    (document.querySelector('test-streamed-fragment-props') as TestStreamedFragmentProps)
      ?.hydrations === 1,
  );
  const props = page.locator('test-streamed-fragment-props');
  await expect(page.locator('footer')).toHaveCount(0);
  await props.evaluate(async (element: TestStreamedFragmentProps) => {
    element.model = { name: 'Live client model' };
    await Promise.resolve();
    element.remove();
    await Promise.resolve();
    document.body.append(element);
  });
  await expect(props.locator('.prop-capture')).toHaveText('Live client model/Stream forest');
  await props.locator('.prop-capture').click();
  await expect(props.locator('.prop-selected')).toHaveText('Live client model/Stream forest');
  await page.waitForLoadState('load');
  await expect(props.locator('.prop-capture')).toHaveText('Live client model/Stream forest');
  expect(errors).toEqual([]);
});
