// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test, type Page, type Request } from '@playwright/test';
import type { LazyOwner } from './element.js';

const isPack = (request: Request): boolean => request.url().includes('/lazy-ui/_webui/packs/');
const isState = (request: Request): boolean => request.url().includes('/lazy-ui/state');

async function ready(page: Page, tags = ['test-lazy-shadow', 'test-lazy-light']): Promise<void> {
  await page.waitForFunction(list => list.every(tag =>
    (document.querySelector(tag) as { $ready?: boolean } | null)?.$ready === true), tags);
}

/** Wait until both owners' idle-triggered regions have been prefetched. */
async function idlePrefetched(page: Page): Promise<void> {
  await page.waitForFunction(() =>
    window.__lazyFixture.registered('test-lazy-shadow', 2) && window.__lazyFixture.registered('test-lazy-light', 2));
}

function packUrl(page: Page, owner: string, ordinal = 0): Promise<string> {
  return page.evaluate(([tag, index]) => window.__lazyFixture.packUrl(tag, index), [owner, ordinal] as const);
}

test('SSR-open regions hydrate in place without pack or state requests', async ({ page }) => {
  await page.addInitScript(() => {
    const descriptor = Object.getOwnPropertyDescriptor(Element.prototype, 'innerHTML')!;
    const captures: string[] = [];
    Object.defineProperty(window, '__templateParses', { value: captures });
    Object.defineProperty(Element.prototype, 'innerHTML', {
      ...descriptor,
      set(value: string) {
        if (this instanceof HTMLTemplateElement) captures.push(String(value));
        descriptor.set!.call(this, value);
      },
    });
  });
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  const requested: string[] = [];
  page.on('request', request => { if (isPack(request) || isState(request)) requested.push(request.url()); });
  const release = Promise.withResolvers<void>();
  await page.route('**/dist/lazy-ui/element.js', async route => {
    await release.promise;
    await route.continue();
  });
  await page.goto('/lazy-ui/fixture.html', { waitUntil: 'commit' });
  await page.locator('test-lazy-shadow .lazy-body').waitFor();
  await page.evaluate(() => {
    const runtime = window as typeof window & { initialLazyNode?: Element | null };
    runtime.initialLazyNode = document.querySelector('test-lazy-shadow')?.shadowRoot?.querySelector('.lazy-body');
  });
  release.resolve();
  await ready(page);
  await idlePrefetched(page);
  expect(await page.evaluate(() => Reflect.get(window, '__templateParses'))).toEqual([]);
  expect(await page.evaluate(() => {
    const runtime = window as typeof window & { initialLazyNode?: Element | null };
    return runtime.initialLazyNode === document.querySelector('test-lazy-shadow')?.shadowRoot?.querySelector('.lazy-body');
  })).toBe(true);
  const openPacks = [await packUrl(page, 'test-lazy-shadow'), await packUrl(page, 'test-lazy-light')];
  expect(openPacks.every(url => url.length > 0)).toBe(true);
  expect(requested.filter(url => openPacks.includes(url) || url.includes('/lazy-ui/state'))).toEqual([]);
  for (const tag of ['test-lazy-shadow', 'test-lazy-light']) {
    await expect(page.locator(`${tag} .message`)).toHaveText('Server state');
    await expect(page.locator(`${tag} .second-root`)).toHaveText('Second inline root');
    await expect(page.locator(`${tag} .lazy-body`)).toHaveCSS('color', 'rgb(17, 34, 51)');
  }
  await page.locator('test-lazy-shadow .increment').click();
  await expect(page.locator('test-lazy-shadow .count')).toHaveText('3');
  await expect(page.locator('test-lazy-light .count')).toHaveText('2');
  await page.locator('test-lazy-shadow .pick').click();
  await expect(page.locator('test-lazy-shadow .chosen')).toHaveText('first');
  expect(await page.evaluate(() => (document.querySelector('test-lazy-shadow') as LazyOwner).inputEl.value)).toBe('Shadow');
  expect(errors).toEqual([]);
});

test('closed regions prefetch on intent and render fresh state on activation', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  const response = await page.goto('/lazy-ui/fixture.html?initial=0');
  const html = await response!.text();
  expect(html).not.toContain('Expanded inline body');
  expect(html).not.toContain('Second inline root');
  await ready(page);
  await expect(page.locator('.lazy-body')).toHaveCount(0);
  const shadowPack = await packUrl(page, 'test-lazy-shadow');
  const lightPack = await packUrl(page, 'test-lazy-light');
  expect(shadowPack).toMatch(/\/lazy-ui\/_webui\/packs\/test-lazy-shadow\.[0-9a-f]{16}\.json$/);
  const requested: string[] = [];
  page.on('request', request => { if (isPack(request) || isState(request)) requested.push(request.url()); });

  const prefetched = page.waitForResponse(shadowPack);
  await page.locator('test-lazy-shadow .open').hover();
  expect((await prefetched).ok()).toBe(true);
  await page.waitForFunction(() => window.__lazyFixture.registered('test-lazy-shadow'));
  await expect(page.locator('.lazy-body')).toHaveCount(0);

  await page.locator('test-lazy-shadow .open').click();
  await expect(page.locator('test-lazy-shadow .message')).toContainText('Fresh test-lazy-shadow');
  await expect(page.locator('test-lazy-shadow .pick')).toHaveText('Fresh row');
  await page.locator('test-lazy-shadow .pick').click();
  await expect(page.locator('test-lazy-shadow .chosen')).toHaveText('fresh');
  expect(requested.filter(url => url === shadowPack)).toHaveLength(1);
  expect(requested.filter(url => url.includes('/lazy-ui/state'))).toHaveLength(1);
  expect(requested).not.toContain(lightPack);
  await expect(page.locator('test-lazy-light .lazy-body')).toHaveCount(0);
  expect(errors).toEqual([]);
});

test('idle regions prefetch after load without rendering or touching intent regions', async ({ page }) => {
  await page.goto('/lazy-ui/fixture.html?initial=0');
  await ready(page);
  await idlePrefetched(page);
  await expect(page.locator('.single-ref')).toHaveCount(0);
  expect(await page.evaluate(() => window.__lazyFixture.registered('test-lazy-light'))).toBe(false);
});

test('$rendered waits for an activated region and the nested region it activates', async ({ page }) => {
  await page.goto('/lazy-ui/fixture.html?initial=0&advanced=1');
  await ready(page);
  const packs: string[] = [];
  page.on('request', request => { if (isPack(request)) packs.push(request.url()); });
  const outer = await packUrl(page, 'test-lazy-shadow');
  const counts = await page.evaluate(async () => {
    const owner = document.querySelector('test-lazy-shadow') as LazyOwner;
    await owner.open();
    const root = owner.shadowRoot!;
    return [root.querySelectorAll('.lazy-body').length, root.querySelectorAll('.advanced').length];
  });
  expect(counts).toEqual([1, 1]);
  const nested = await packUrl(page, 'test-lazy-shadow', 1);
  expect(nested).not.toBe('');
  expect(packs).toContain(outer);
  expect(packs).toContain(nested);
  await expect(page.locator('test-lazy-shadow .advanced')).toHaveText('Nested lazy body');
});

test('closing before the pack arrives keeps the region unmounted', async ({ page }) => {
  await page.goto('/lazy-ui/fixture.html?initial=0&delay=0');
  await ready(page);
  const url = await packUrl(page, 'test-lazy-shadow');
  const release = Promise.withResolvers<void>();
  await page.route(url, async route => {
    await release.promise;
    await route.continue();
  });
  const opened = page.evaluate(() => (document.querySelector('test-lazy-shadow') as LazyOwner).open());
  await page.waitForFunction(() => (document.querySelector('test-lazy-shadow') as LazyOwner).opened);
  await page.locator('test-lazy-shadow .close').click();
  release.resolve();
  await opened;
  await page.waitForFunction(() => window.__lazyFixture.registered('test-lazy-shadow'));
  await expect(page.locator('test-lazy-shadow .lazy-body')).toHaveCount(0);
  await expect(page.locator('test-lazy-shadow .loading')).toHaveCount(0);
});

test('a failed pack rejects $rendered and the next activation retries it', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto('/lazy-ui/fixture.html?initial=0&delay=0');
  await ready(page);
  const url = await packUrl(page, 'test-lazy-light');
  let failures = 1;
  await page.route(url, route => failures-- > 0
    ? route.fulfill({ status: 503, body: 'unavailable' })
    : route.continue());
  await page.evaluate(() => (document.querySelector('test-lazy-light') as LazyOwner).open());
  await expect(page.locator('test-lazy-light .error')).toContainText('failed with HTTP 503');
  await expect(page.locator('test-lazy-light .lazy-body')).toHaveCount(0);
  await page.evaluate(() => (document.querySelector('test-lazy-light') as LazyOwner).open());
  await expect(page.locator('test-lazy-light .lazy-body')).toHaveCount(1);
  await expect(page.locator('test-lazy-light .error')).toHaveCount(0);
  expect(errors).toEqual([]);
});

test('state failures remain local and reopening does not reuse stale instance state', async ({ page }) => {
  await page.goto('/lazy-ui/fixture.html?initial=0&fail=1');
  await ready(page);
  await page.evaluate(() => (document.querySelector('test-lazy-light') as LazyOwner).open());
  await expect(page.locator('test-lazy-light .error')).toContainText('could not load lazy state');
  await expect(page.locator('test-lazy-light .lazy-body')).toHaveCount(0);
  await page.evaluate(() => history.replaceState({}, '', '/lazy-ui/fixture.html?initial=0'));
  await page.evaluate(() => (document.querySelector('test-lazy-light') as LazyOwner).open());
  const first = await page.locator('test-lazy-light .message').textContent();
  await page.locator('test-lazy-light .close').click();
  await page.evaluate(() => (document.querySelector('test-lazy-light') as LazyOwner).open());
  await expect(page.locator('test-lazy-light .message')).not.toHaveText(first!);
});

test('closed regions are released at idle, become collectable, and reload when reopened', async ({ page }) => {
  await page.goto('/lazy-ui/fixture.html?initial=0&delay=0');
  await ready(page);
  await idlePrefetched(page);
  const owner = page.locator('test-lazy-shadow');
  await owner.evaluate(element => (element as LazyOwner).open());
  await expect(page.locator('test-lazy-shadow .lazy-body')).toHaveCount(1);
  const openInventory = await page.evaluate(() => window.__lazyFixture.inventory());
  await page.evaluate(() => window.__lazyFixture.track('test-lazy-shadow'));
  const session = await page.context().newCDPSession(page);
  await session.send('HeapProfiler.collectGarbage');
  const before = await session.send('Runtime.getHeapUsage');

  await page.locator('test-lazy-shadow .close').click();
  await page.waitForFunction(() => !window.__lazyFixture.registered('test-lazy-shadow'));
  expect(await page.evaluate(() => window.__lazyFixture.inventory())).not.toBe(openInventory);
  let retained = true;
  for (let attempt = 0; attempt < 5 && retained; attempt++) {
    await page.evaluate(() => new Promise<void>(resolve => setTimeout(resolve, 0)));
    await session.send('HeapProfiler.collectGarbage');
    retained = await page.evaluate(() => window.__lazyFixture.retained());
  }
  const after = await session.send('Runtime.getHeapUsage');
  console.log(JSON.stringify({
    lazyMemory: { releasedBodyRetained: retained, beforeUsedHeap: before.usedSize, afterUsedHeap: after.usedSize },
  }));
  expect(retained).toBe(false);
  await expect(owner).toHaveCount(1);
  expect(await owner.evaluate(element => (element as LazyOwner).inputEl === undefined)).toBe(true);

  const reload = page.waitForRequest(await packUrl(page, 'test-lazy-shadow'));
  await owner.evaluate(element => (element as LazyOwner).open());
  await reload;
  await expect(page.locator('test-lazy-shadow .lazy-body')).toHaveCount(1);
  expect(await page.evaluate(() => window.__lazyFixture.inventory())).toBe(openInventory);
  await session.detach();
});

test('template collection keeps live regions and closed regions load again', async ({ page }) => {
  await page.goto('/lazy-ui/fixture.html');
  await ready(page);
  await page.evaluate(() => window.__lazyFixture.collect());
  await page.locator('test-lazy-light .increment').click();
  await expect(page.locator('test-lazy-light .count')).toHaveText('3');
  await page.locator('test-lazy-shadow .close').click();
  await page.locator('test-lazy-light .close').click();
  expect(await page.evaluate(() => (document.querySelector('test-lazy-light') as LazyOwner).inputEl === undefined)).toBe(true);
  await page.evaluate(() => window.__lazyFixture.collect());
  await page.evaluate(() => (document.querySelector('test-lazy-light') as LazyOwner).open());
  await expect(page.locator('test-lazy-light .message')).toContainText('Fresh test-lazy-light');
  await expect(page.locator('test-lazy-light .second-root')).toHaveCount(1);
});

test('a ref on the SSR body root belongs to its owner and is released on removal', async ({ page }) => {
  await page.goto('/lazy-ui/fixture.html?initial=0&single=1');
  await ready(page);
  for (const tag of ['test-lazy-shadow', 'test-lazy-light']) {
    expect(await page.locator(tag).evaluate(element => {
      const owner = element as LazyOwner;
      return owner.singleEl?.value;
    })).toBe('Server state');
    await page.locator(tag).evaluate(element => { (element as LazyOwner).single = false; });
    await expect(page.locator(`${tag} .single-ref`)).toHaveCount(0);
    expect(await page.locator(tag).evaluate(element => (element as LazyOwner).singleEl)).toBeUndefined();
  }
});

test('a native dialog hydrates its initially rendered body without a state request', async ({ page }) => {
  let states = 0;
  page.on('request', request => { if (isState(request)) states++; });
  await page.goto('/lazy-ui/fixture.html?dialog=1');
  await ready(page, ['test-lazy-dialog']);
  await expect(page.locator('test-lazy-dialog dialog')).toBeVisible();
  await expect(page.locator('test-lazy-dialog .message')).toHaveText('Server state');
  expect(states).toBe(0);
  await page.locator('test-lazy-dialog .close').click();
  await expect(page.locator('test-lazy-dialog dialog')).not.toBeVisible();
  await expect(page.locator('test-lazy-dialog .message')).toHaveCount(0);
});

test('a closed native dialog loads its body and Escape releases its state', async ({ page }) => {
  await page.goto('/lazy-ui/fixture.html?dialog=1&initial=0');
  await ready(page, ['test-lazy-dialog']);
  await page.locator('test-lazy-dialog .open').click();
  await expect(page.locator('test-lazy-dialog .message')).toContainText('Fresh test-lazy-dialog');
  await page.keyboard.press('Escape');
  await expect(page.locator('test-lazy-dialog dialog')).not.toBeVisible();
  await expect(page.locator('test-lazy-dialog .message')).toHaveCount(0);
  expect(await page.locator('test-lazy-dialog').evaluate(element => (element as LazyOwner).message)).toBe('');
});
