// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test, type Page } from '@playwright/test';
import type { LazyOwner } from './element.js';

async function ready(page: Page): Promise<void> {
  await page.waitForFunction(() =>
    ['test-lazy-shadow', 'test-lazy-light'].every(tag =>
      (document.querySelector(tag) as { $ready?: boolean } | null)?.$ready === true),
  );
}

test('initial SSR hydrates inline bodies without a partial request or replacement host', async ({ page }) => {
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
  let requests = 0;
  page.on('request', request => {
    if (request.url().includes('/lazy-ui/state') || request.url().includes('/lazy-ui/resources')) requests++;
  });
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
  expect(await page.evaluate(() => Reflect.get(window, '__templateParses'))).toEqual([]);
  expect(await page.evaluate(() => {
    const runtime = window as typeof window & { initialLazyNode?: Element | null };
    return runtime.initialLazyNode === document.querySelector('test-lazy-shadow')?.shadowRoot?.querySelector('.lazy-body');
  })).toBe(true);
  expect(requests).toBe(0);
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

test('inactive bodies are absent, preload creates no UI, and open receives fresh state', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  const response = await page.goto('/lazy-ui/fixture.html?initial=0');
  const html = await response!.text();
  expect(html).not.toContain('Expanded inline body');
  expect(html).not.toContain('Second inline root');
  await ready(page);
  await expect(page.locator('.lazy-body')).toHaveCount(0);
  let stateRequests = 0;
  page.on('request', request => { if (request.url().includes('/lazy-ui/state')) stateRequests++; });
  const preload = page.waitForResponse(response => response.url().includes('/lazy-ui/resources'));
  await page.locator('test-lazy-shadow .open').hover();
  expect((await preload).ok()).toBe(true);
  await page.waitForFunction(() => window.__lazyFixture.moduleLoads() === 1);
  await expect(page.locator('.lazy-body')).toHaveCount(0);
  expect(stateRequests).toBe(0);

  await page.locator('test-lazy-shadow .open').click();
  await expect(page.locator('test-lazy-shadow .message')).toContainText('Fresh test-lazy-shadow');
  await expect(page.locator('test-lazy-shadow .pick')).toHaveText('Fresh row');
  await page.locator('test-lazy-shadow .pick').click();
  await expect(page.locator('test-lazy-shadow .chosen')).toHaveText('fresh');
  expect(stateRequests).toBe(1);
  await expect(page.locator('test-lazy-light .lazy-body')).toHaveCount(0);
  expect(errors).toEqual([]);
});

test('the streamed state can activate a nested lazy body without another resource request', async ({ page }) => {
  await page.goto('/lazy-ui/fixture.html?initial=0&advanced=1');
  await ready(page);
  let resources = 0;
  let states = 0;
  page.on('request', request => {
    if (request.url().includes('/lazy-ui/resources')) resources++;
    if (request.url().includes('/lazy-ui/state')) states++;
  });
  await page.evaluate(() => (document.querySelector('test-lazy-shadow') as LazyOwner).open());
  await expect(page.locator('test-lazy-shadow .advanced')).toHaveText('Nested lazy body');
  expect(resources).toBe(0);
  expect(states).toBe(1);
});

test('closing cancels an unfinished open and late state does not mount the body', async ({ page }) => {
  await page.goto('/lazy-ui/fixture.html?initial=0&delay=350');
  await ready(page);
  const request = page.waitForRequest(request => request.url().includes('/lazy-ui/state'));
  await page.evaluate(() => { void (document.querySelector('test-lazy-shadow') as LazyOwner).open(); });
  await request;
  await page.locator('test-lazy-shadow .close').click();
  await expect(page.locator('test-lazy-shadow .loading')).toHaveCount(0);
  await page.waitForTimeout(400);
  await expect(page.locator('test-lazy-shadow .lazy-body')).toHaveCount(0);
});

test('state failures remain local and reopening does not reuse stale instance state', async ({ page }) => {
  await page.goto('/lazy-ui/fixture.html?initial=0&fail=1');
  await ready(page);
  await page.evaluate(() => (document.querySelector('test-lazy-light') as LazyOwner).open());
  await expect(page.locator('test-lazy-light .error')).toContainText('could not load lazy state');
  await expect(page.locator('test-lazy-light .lazy-body')).toHaveCount(0);
  expect(page.url()).toContain('fail=1');
  await page.evaluate(() => history.replaceState({}, '', '/lazy-ui/fixture.html?initial=0'));
  await page.evaluate(() => (document.querySelector('test-lazy-light') as LazyOwner).open());
  const first = await page.locator('test-lazy-light .message').textContent();
  await page.locator('test-lazy-light .close').click();
  await page.evaluate(() => (document.querySelector('test-lazy-light') as LazyOwner).open());
  await expect(page.locator('test-lazy-light .message')).not.toHaveText(first!);
});

test('collection preserves live bodies and closed bodies can be loaded again', async ({ page }) => {
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
  page.on('request', request => { if (request.url().includes('/lazy-ui/state')) states++; });
  await page.goto('/lazy-ui/fixture.html?dialog=1');
  await page.waitForFunction(() => Reflect.get(document.querySelector('test-lazy-dialog') ?? {}, '$ready') === true);
  await expect(page.locator('test-lazy-dialog dialog')).toBeVisible();
  await expect(page.locator('test-lazy-dialog .message')).toHaveText('Server state');
  expect(states).toBe(0);
  await page.locator('test-lazy-dialog .close').click();
  await expect(page.locator('test-lazy-dialog dialog')).not.toBeVisible();
  await expect(page.locator('test-lazy-dialog .message')).toHaveCount(0);
});

test('a closed native dialog loads an inline body and Escape releases its state', async ({ page }) => {
  await page.goto('/lazy-ui/fixture.html?dialog=1&initial=0');
  await page.waitForFunction(() => Reflect.get(document.querySelector('test-lazy-dialog') ?? {}, '$ready') === true);
  await page.locator('test-lazy-dialog .open').click();
  await expect(page.locator('test-lazy-dialog .message')).toContainText('Fresh test-lazy-dialog');
  await page.keyboard.press('Escape');
  await expect(page.locator('test-lazy-dialog dialog')).not.toBeVisible();
  await expect(page.locator('test-lazy-dialog .message')).toHaveCount(0);
  expect(await page.locator('test-lazy-dialog').evaluate(element => (element as LazyOwner).message)).toBe('');
});

test('reports closed-body collection while its owning component remains alive', async ({ page }) => {
  await page.goto('/lazy-ui/fixture.html');
  await ready(page);
  await page.evaluate(() => window.__lazyFixture.track('test-lazy-shadow'));
  const session = await page.context().newCDPSession(page);
  await session.send('HeapProfiler.collectGarbage');
  const before = await session.send('Runtime.getHeapUsage');
  await page.locator('test-lazy-shadow .close').click();
  await page.evaluate(() => window.__lazyFixture.collect());
  let retained = true;
  for (let attempt = 0; attempt < 3 && retained; attempt++) {
    await page.evaluate(() => new Promise<void>(resolve => setTimeout(resolve, 0)));
    await session.send('HeapProfiler.collectGarbage');
    retained = await page.evaluate(() => window.__lazyFixture.retained());
  }
  const after = await session.send('Runtime.getHeapUsage');
  console.log(JSON.stringify({
    lazyMemory: {
      closedBodyRetained: retained,
      beforeUsedHeap: before.usedSize,
      afterUsedHeap: after.usedSize,
    },
  }));
  await expect(page.locator('test-lazy-shadow')).toHaveCount(1);
  await expect(page.locator('test-lazy-shadow .lazy-body')).toHaveCount(0);
  expect(await page.evaluate(() => (document.querySelector('test-lazy-shadow') as LazyOwner).inputEl === undefined)).toBe(true);
  await session.detach();
});
