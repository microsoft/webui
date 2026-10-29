// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test, type Page } from '@playwright/test';
import type { LinkOwner } from './element.js';

const REGION = 'test-link-owner::lazy:@0';
const STYLESHEET = '/lazy-link/test-link-chip.css';
const CSS = '.chip { color: rgb(170, 0, 0); }';

interface Probe {
  /** Requests in the order they were sent, with their initial Chromium priority. */
  requests: [path: string, priority: string][];
  /** Stylesheet requests that reached the network. */
  stylesheets(): number;
  /** Path of the region's pack. */
  packPath(): Promise<string>;
  /** Hold the region's pack until the returned function runs. */
  holdPack(): Promise<() => void>;
}

async function probe(page: Page): Promise<Probe> {
  const requests: [string, string][] = [];
  const session = await page.context().newCDPSession(page);
  await session.send('Network.enable');
  session.on('Network.requestWillBeSent', event => {
    requests.push([new URL(event.request.url).pathname, event.request.initialPriority]);
  });
  let stylesheets = 0;
  await page.route(`**${STYLESHEET}`, route => {
    stylesheets++;
    return route.fulfill({ contentType: 'text/css', body: CSS });
  });
  const packPath = async (): Promise<string> => {
    const path = await page.evaluate(region => {
      const url = window.__webui?.templates?.['test-link-owner']?.lp?.[region];
      return url ? new URL(url, document.baseURI).pathname : '';
    }, REGION);
    expect(path).toMatch(/\/lazy-link\/_webui\/packs\/test-link-owner\.[0-9a-f]{16}\.json$/);
    return path;
  };
  return {
    requests,
    stylesheets: () => stylesheets,
    packPath,
    async holdPack() {
      const pack = Promise.withResolvers<void>();
      await page.route(`**${await packPath()}`, async route => {
        await pack.promise;
        await route.continue();
      });
      return pack.resolve;
    },
  };
}

/** Keep the window loading until the returned function runs, by holding the page's image. */
async function holdWindowLoad(page: Page): Promise<() => void> {
  const image = Promise.withResolvers<void>();
  await page.route('**/lazy-link/slow.gif', async route => {
    await image.promise;
    await route.fulfill({ status: 204 });
  });
  return image.resolve;
}

async function ready(page: Page): Promise<void> {
  await page.waitForFunction(() =>
    (document.querySelector('test-link-owner') as { $ready?: boolean } | null)?.$ready === true);
}

/** Prefetch the region on intent and wait until its pack has installed. */
async function prefetch(page: Page): Promise<void> {
  await page.locator('test-link-owner .open').hover();
  await page.waitForFunction(region => window.__webui?.templates?.[region] !== undefined, REGION);
}

/** Stylesheet preloads in the document head, with their fetch priority. */
function stylesheetPreloads(page: Page): Promise<[string, string][]> {
  return page.evaluate(() => [...document.head.querySelectorAll<HTMLLinkElement>('link[rel="preload"][as="style"]')]
    .map(link => [new URL(link.href).pathname, link.fetchPriority]));
}

test('intent before the window loads requests only the pack, and its stylesheets start once it has', async ({ page }) => {
  const network = await probe(page);
  const finishLoading = await holdWindowLoad(page);
  await page.goto('/lazy-link/fixture.html', { waitUntil: 'commit' });
  await ready(page);
  const pack = await network.packPath();
  const sent = network.requests.length;
  await prefetch(page);
  expect(await page.evaluate(() => document.readyState)).not.toBe('complete');
  const speculative = network.requests.slice(sent).filter(([path]) => path !== '/favicon.ico');
  expect(speculative.filter(([, priority]) => priority === 'High' || priority === 'VeryHigh')).toEqual([]);
  expect(speculative).toEqual([[pack, 'Low']]);
  expect(await stylesheetPreloads(page)).toEqual([]);

  const styled = page.waitForRequest(`**${STYLESHEET}`);
  finishLoading();
  await styled;
  expect(await page.evaluate(() => document.readyState)).toBe('complete');
  await page.locator('test-link-owner .open').click();
  await expect(page.locator('test-link-chip .chip')).toHaveCSS('color', 'rgb(170, 0, 0)');
  expect(network.stylesheets(), 'the mount reuses the stylesheet prepared after load').toBe(1);
});

test('an activation before the window loads styles a region that installed without its stylesheets', async ({ page }) => {
  const network = await probe(page);
  const finishLoading = await holdWindowLoad(page);
  await page.goto('/lazy-link/fixture.html', { waitUntil: 'commit' });
  await ready(page);
  await prefetch(page);
  const stylesheet = Promise.withResolvers<void>();
  await page.route(`**${STYLESHEET}`, async route => {
    await stylesheet.promise;
    await route.fallback();
  });
  const requested = page.waitForRequest(`**${STYLESHEET}`);
  await page.locator('test-link-owner .open').click();
  // The stylesheet starts at once and the guarded link keeps the body hidden until it applies.
  await requested;
  expect(await page.evaluate(() => document.readyState)).not.toBe('complete');
  const chip = page.locator('test-link-chip .chip');
  await expect(chip).toHaveCount(1);
  await expect(chip).toBeHidden();
  stylesheet.resolve();
  await expect(chip).toBeVisible();
  await expect(chip).toHaveCSS('color', 'rgb(170, 0, 0)');
  expect(network.stylesheets()).toBe(1);
  finishLoading();
});

test('activation preloads the region stylesheets beside its pack at the default priority', async ({ page }) => {
  const network = await probe(page);
  await page.goto('/lazy-link/fixture.html');
  await ready(page);
  const releasePack = await network.holdPack();
  const requested = page.waitForRequest(`**${STYLESHEET}`);
  // Activate without hovering, so no intent prefetch runs first.
  await page.evaluate(() => (document.querySelector('test-link-owner') as LinkOwner).show());
  await requested;
  expect(await stylesheetPreloads(page)).toEqual([[STYLESHEET, 'auto']]);
  expect(network.requests.filter(([path]) => path === STYLESHEET)).toEqual([[STYLESHEET, 'VeryHigh']]);

  releasePack();
  await expect(page.locator('test-link-chip .chip')).toHaveCSS('color', 'rgb(170, 0, 0)');
  expect(network.stylesheets()).toBe(1);
});
