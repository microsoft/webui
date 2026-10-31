// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test, type Page } from '@playwright/test';

interface Recorder {
  /** Pack requests the page has started. */
  packs(): Promise<string[]>;
  /** The composed path of each `pointerover` the document heard. */
  overs(): Promise<string>;
}

/** Record pack fetches as the runtime starts them, and what the document hears. */
async function record(page: Page): Promise<Recorder> {
  await page.addInitScript(() => {
    const packs: string[] = [];
    const overs: string[] = [];
    Object.defineProperty(window, '__intentProbe', { value: { packs, overs } });
    const fetch = window.fetch;
    window.fetch = (input, init) => {
      const url = String(input instanceof Request ? input.url : input);
      if (url.includes('/lazy-intent/_webui/packs/')) packs.push(url);
      return fetch(input, init);
    };
    document.addEventListener('pointerover', event => {
      overs.push(event.composedPath()
        .filter((node): node is Element => node instanceof Element)
        .map(node => node.localName + (node.className ? `.${node.className}` : ''))
        .join('>'));
    }, { capture: true });
  });
  const probe = (): Promise<{ packs: string[]; overs: string[] }> =>
    page.evaluate(() => Reflect.get(window, '__intentProbe') as { packs: string[]; overs: string[] });
  return {
    packs: async () => (await probe()).packs,
    overs: async () => (await probe()).overs.join('\n'),
  };
}

/** Open the page and wait for the nested owners to hydrate. */
async function open(page: Page): Promise<void> {
  await page.goto('/lazy-intent/fixture.html');
  await page.waitForFunction(() => ['test-intent-owner', 'test-intent-later'].every(tag =>
    (document.querySelector('test-intent-app')?.shadowRoot?.querySelector(tag) as
      { $ready?: boolean } | null)?.$ready === true));
}

async function center(page: Page, selector: string): Promise<[number, number]> {
  const box = await page.locator(selector).boundingBox();
  expect(box).not.toBeNull();
  return [box!.x + box!.width / 2, box!.y + box!.height / 2];
}

test('a pointer that moves to a nested owner from a sibling in its shadow tree prefetches the owner', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  const probe = await record(page);
  await open(page);
  const tile = await center(page, 'test-intent-app .tile');
  const owner = await center(page, 'test-intent-owner .open');

  await page.mouse.move(...tile);
  await expect.poll(probe.overs).toContain('button.tile>test-intent-app');
  expect(await probe.packs(), 'the owner is not on the path into the tile').toEqual([]);

  // The move stays inside the app's shadow tree, so its path stops before the app.
  await page.mouse.move(...owner);
  await expect.poll(probe.packs).toHaveLength(1);
  expect(await probe.overs(), 'the document never hears the move').not.toContain('test-intent-owner');

  await page.locator('test-intent-owner .open').click();
  await expect(page.locator('test-intent-owner .panel')).toHaveText('Settings panel');
  expect(await probe.packs(), 'activation reuses the prefetched pack').toHaveLength(1);
  expect(errors).toEqual([]);
});

test('focus that moves to a nested owner from a sibling in its shadow tree prefetches the owner', async ({ page }) => {
  const probe = await record(page);
  await open(page);
  await page.locator('test-intent-app .tile').focus();
  expect(await probe.packs()).toEqual([]);
  await page.keyboard.press('Tab');
  await expect(page.locator('test-intent-owner .open')).toBeFocused();
  await expect.poll(probe.packs).toHaveLength(1);
});

test('a region that a block reveals after its owner mounted prefetches when the pointer enters from a sibling', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  const probe = await record(page);
  await open(page);
  // The owner hydrated with the block closed, so its lazy region arms only now.
  await page.evaluate(() => {
    const later = document.querySelector('test-intent-app')!.shadowRoot!.querySelector('test-intent-later');
    (later as HTMLElement & { shown: boolean }).shown = true;
  });
  await expect(page.locator('test-intent-later .open')).toBeVisible();
  const tile = await center(page, 'test-intent-app .tile');
  const later = await center(page, 'test-intent-later .open');

  await page.mouse.move(...tile);
  await expect.poll(probe.overs).toContain('button.tile>test-intent-app');
  expect(await probe.packs()).toEqual([]);
  await page.mouse.move(...later);
  await expect.poll(probe.packs).toEqual([expect.stringContaining('/test-intent-later.')]);
  expect(await probe.overs(), 'the document never hears the move').not.toContain('test-intent-later');
  await expect(page.locator('test-intent-later .panel')).toHaveCount(0);
  expect(errors).toEqual([]);
});
