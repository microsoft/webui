// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test, type Page } from '@playwright/test';
import type { Capture, ConnectivityList, Item } from './element.js';

async function openFixture(page: Page): Promise<void> {
  await page.goto('/ssr-repeat-connectivity/fixture.html');
  await page.waitForFunction(() => {
    const list = document.querySelector('connectivity-list');
    return list && Reflect.get(list, '$ready') === true;
  });
}

test('adopts complete SSR ranges without disconnecting children on cold or warm loads', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));

  for (const load of ['cold', 'warm']) {
    await test.step(load, async () => {
      await openFixture(page);
      for (const section of ['compact', 'formatted', 'fragment', 'unkeyed', 'structural', 'conditional']) {
        await expect(page.locator(`#${section} connectivity-probe`)).toHaveText(['Alpha', 'Beta']);
      }
      await expect(page.locator('#nested connectivity-probe')).toHaveText(['Alpha child', 'Beta child']);
      await expect(page.locator('#fragment b')).toHaveText(['Alpha raw', 'Beta raw']);
      await expect(page.locator('connectivity-probe').first()).toHaveCSS('color', 'rgb(12, 34, 56)');

      expect(await page.evaluate(() => {
        const capture = Reflect.get(window, '__connectivityCapture') as Capture;
        return {
          disconnects: capture.disconnects,
          sameNodesAndParents: capture.nodes.every(({ node, parent }) =>
            node.isConnected && node.parentNode === parent),
          linkedShadowStyles: capture.probes.every(probe =>
            probe.shadowRoot?.querySelector('link[rel="stylesheet"]')?.isConnected),
        };
      })).toEqual({ disconnects: [], sameNodesAndParents: true, linkedShadowStyles: true });
    });
  }
  expect(errors).toEqual([]);
});

test('keeps keyed and unkeyed ownership through updates, insertion, removal and clear', async ({ page }) => {
  await openFixture(page);
  await page.evaluate(() => {
    const list = document.querySelector('connectivity-list') as ConnectivityList;
    list.items = [...list.items].reverse();
  });
  await expect(page.locator('#formatted connectivity-probe')).toHaveText(['Beta', 'Alpha']);
  await expect(page.locator('#unkeyed connectivity-probe')).toHaveText(['Beta', 'Alpha']);
  expect(await page.evaluate(() => {
    const capture = Reflect.get(window, '__connectivityCapture') as Capture;
    const root = document.querySelector('connectivity-list')!.shadowRoot!;
    const original = (id: string) => capture.probes.filter(probe => probe.parentElement?.id === id);
    const keyed = original('formatted');
    const unkeyed = original('unkeyed');
    const keyedNow = root.querySelectorAll('#formatted connectivity-probe');
    const unkeyedNow = root.querySelectorAll('#unkeyed connectivity-probe');
    return {
      keyed: keyedNow[0] === keyed[1] && keyedNow[1] === keyed[0],
      unkeyed: unkeyedNow[0] === unkeyed[0] && unkeyedNow[1] === unkeyed[1],
    };
  })).toEqual({ keyed: true, unkeyed: true });

  await page.evaluate(() => {
    const list = document.querySelector('connectivity-list') as ConnectivityList;
    list.items = list.items.map(item => ({
      ...item, title: `${item.title} updated`, detail: `${item.detail} updated`,
      html: `<b>${item.title} raw updated</b>`,
    }));
  });
  await expect(page.locator('#fragment connectivity-probe')).toHaveText(['Beta updated', 'Alpha updated']);
  await expect(page.locator('#fragment .detail')).toHaveText(['Beta detail updated', 'Alpha detail updated']);
  await expect(page.locator('#fragment b')).toHaveText(['Beta raw updated', 'Alpha raw updated']);
  await expect(page.locator('#fragment')).toContainText('tail Alpha updated');
  await expect(page.locator('#unkeyed')).toContainText('Lead Beta updated');
  await expect(page.locator('#structural')).toContainText('conditional Alpha updated');
  await expect(page.locator('#fragment .sentinel')).toHaveText('Fragment tail');

  await page.evaluate(() => {
    const list = document.querySelector('connectivity-list') as ConnectivityList;
    const item: Item = {
      id: 'c', title: 'Gamma', detail: 'Gamma detail', html: '<b>Gamma raw</b>',
      children: [{ id: 'c1', title: 'Gamma child' }],
    };
    list.items = [item, ...list.items];
  });
  await expect(page.locator('#formatted connectivity-probe')).toHaveText(['Gamma', 'Beta updated', 'Alpha updated']);
  await expect(page.locator('#nested connectivity-probe')).toHaveText(['Gamma child', 'Beta child', 'Alpha child']);
  await page.evaluate(() => {
    const list = document.querySelector('connectivity-list') as ConnectivityList;
    list.items = [list.items[0], list.items[2]];
  });
  await expect(page.locator('#formatted connectivity-probe')).toHaveText(['Gamma', 'Alpha updated']);
  await page.locator('#fragment button').last().click();
  await expect(page.locator('output')).toHaveText('a');

  await page.evaluate(() => {
    const list = document.querySelector('connectivity-list') as ConnectivityList;
    Reflect.set(window, '__removedButton', list.shadowRoot!.querySelector('#fragment button'));
    list.items = [];
  });
  await expect(page.locator('connectivity-probe')).toHaveCount(0);
  expect(await page.evaluate(() => {
    const capture = Reflect.get(window, '__connectivityCapture') as Capture;
    return capture.repeatNodes.filter(node => node.parentNode !== null).map(node => ({
      node: node.nodeName, text: node.textContent, parent: node.parentNode?.nodeName,
      connected: node.isConnected,
    }));
  })).toEqual([]);
  await expect(page.locator('.sentinel')).toHaveText([
    'Compact tail', 'Formatted tail', 'Fragment tail', 'Unkeyed tail',
    'Nested tail', 'Structural tail', 'Conditional tail',
  ]);
  await page.evaluate(() => {
    const list = document.querySelector('connectivity-list') as ConnectivityList;
    list.selected = 'reset';
    (Reflect.get(window, '__removedButton') as HTMLButtonElement).click();
  });
  await expect(page.locator('output')).toHaveText('reset');
});

test('tears down and recreates conditional fragments without claiming neighboring content', async ({ page }) => {
  await openFixture(page);
  await page.evaluate(() => {
    (document.querySelector('connectivity-list') as ConnectivityList).visible = false;
  });
  await expect(page.locator('#structural connectivity-probe')).toHaveCount(0);
  await expect(page.locator('#conditional connectivity-probe')).toHaveCount(0);
  await expect(page.locator('#conditional .detail')).toHaveCount(0);
  await expect(page.locator('#structural .detail')).toHaveText(['Alpha detail', 'Beta detail']);
  await expect(page.locator('#conditional .sentinel')).toHaveText('Conditional tail');
  await page.evaluate(() => {
    (document.querySelector('connectivity-list') as ConnectivityList).visible = true;
  });
  await expect(page.locator('#structural connectivity-probe')).toHaveText(['Alpha', 'Beta']);
  await expect(page.locator('#conditional connectivity-probe')).toHaveText(['Alpha', 'Beta']);
  await expect(page.locator('#conditional .detail')).toHaveText('Conditional detail');
});
