// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test, type Page } from '@playwright/test';

const SHARED_TAG = 'test-shared-link-card';
const SHARED_COLOR = 'rgb(11, 22, 33)';

async function openFixture(page: Page): Promise<void> {
  await page.goto('/component-link-sharing/fixture.html');
  await page.waitForFunction(() => typeof window.__inspectLinkCards === 'function');
}

async function register(page: Page, tag: string, prepare: boolean): Promise<void> {
  await page.evaluate(
    ([name, warm]) => window.__registerLinkCards?.(name, warm),
    [tag, prepare] as const,
  );
}

/** Mount and inspect in one task, before any frame, load event, or microtask. */
async function mountNow(page: Page, tag: string, count: number) {
  return page.evaluate(([name, total]) => {
    window.__mountLinkCards?.(name, total);
    return window.__inspectLinkCards?.(name);
  }, [tag, count] as const);
}

async function inspect(page: Page, tag: string) {
  return page.evaluate(name => window.__inspectLinkCards?.(name), tag);
}

test.describe('component link sharing', () => {
  test('shares one sheet across a prepared mount pass after one native link', async ({
    page,
  }) => {
    let requests = 0;
    page.on('request', request => {
      if (new URL(request.url()).pathname === '/component-link-sharing/shared.css') {
        requests += 1;
      }
    });
    await openFixture(page);
    await register(page, SHARED_TAG, true);

    // The first root keeps its native link. The second promotes that link's
    // authorized CSSOM once, and every later root adopts the shared sheet
    // without a link, guard, or load wait.
    await expect(mountNow(page, SHARED_TAG, 20)).resolves.toEqual({
      activeLinks: 1,
      adopted: 19,
      cards: 20,
      colors: [SHARED_COLOR],
      distinctSheets: 1,
      guards: 0,
      hidden: 0,
      links: 1,
      sameSheetAsFirstBatch: true,
    });
    expect(requests).toBe(1);
  });

  test('keeps adopting the shared sheet after the preload is released', async ({
    page,
  }) => {
    await openFixture(page);
    await register(page, SHARED_TAG, true);
    await mountNow(page, SHARED_TAG, 2);
    await expect.poll(() => page.evaluate(() => {
      return document.head.querySelectorAll('link[rel="preload"]').length;
    })).toBe(0);

    // A later root's native load is a memory cache hit with no new resource
    // timing entry, so only the shared sheet can style it without a guard.
    const report = await page.evaluate(name => {
      window.__removeLinkCards?.(name);
      window.__mountLinkCards?.(name, 20);
      return window.__inspectLinkCards?.(name);
    }, SHARED_TAG);
    expect(report).toEqual({
      activeLinks: 0,
      adopted: 20,
      cards: 20,
      colors: [SHARED_COLOR],
      distinctSheets: 1,
      guards: 0,
      hidden: 0,
      links: 0,
      sameSheetAsFirstBatch: true,
    });
  });

  test('promotes on the second use after a lone first mount', async ({ page }) => {
    await openFixture(page);
    await register(page, SHARED_TAG, true);

    // A component used once costs no more than its native link.
    await expect(mountNow(page, SHARED_TAG, 1)).resolves.toEqual({
      activeLinks: 1,
      adopted: 0,
      cards: 1,
      colors: [SHARED_COLOR],
      distinctSheets: 0,
      guards: 0,
      hidden: 0,
      links: 1,
      sameSheetAsFirstBatch: true,
    });

    // Later roots promote the first root's loaded link once, using the
    // authorization recorded for the Document's response. The visible first
    // root is never switched.
    await expect(mountNow(page, SHARED_TAG, 2)).resolves.toEqual({
      activeLinks: 1,
      adopted: 2,
      cards: 3,
      colors: [SHARED_COLOR],
      distinctSheets: 1,
      guards: 0,
      hidden: 0,
      links: 1,
      sameSheetAsFirstBatch: true,
    });
  });

  test('promotes from a later root when the first root is gone', async ({ page }) => {
    await openFixture(page);
    await register(page, SHARED_TAG, true);
    await mountNow(page, SHARED_TAG, 1);

    await page.evaluate(name => {
      window.__removeLinkCards?.(name);
      window.__mountLinkCards?.(name, 3);
    }, SHARED_TAG);
    await expect.poll(() => inspect(page, SHARED_TAG)).toMatchObject({
      activeLinks: 0,
      adopted: 3,
      cards: 3,
      colors: [SHARED_COLOR],
      distinctSheets: 1,
      guards: 0,
      hidden: 0,
    });
  });

  test('shares one sheet among guarded cold mounts once they load', async ({ page }) => {
    let releaseStylesheet!: () => void;
    let stylesheetRequested!: () => void;
    const stylesheetGate = new Promise<void>(resolve => {
      releaseStylesheet = resolve;
    });
    const requestSeen = new Promise<void>(resolve => {
      stylesheetRequested = resolve;
    });
    await page.route('**/component-link-sharing/cold.css', async route => {
      stylesheetRequested();
      await stylesheetGate;
      await route.continue();
    });
    await openFixture(page);
    await register(page, 'test-cold-link-card', false);

    await mountNow(page, 'test-cold-link-card', 5);
    await requestSeen;
    await expect(page.evaluate(async name => {
      await new Promise<void>(resolve =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve()))
      );
      return window.__inspectLinkCards?.(name);
    }, 'test-cold-link-card')).resolves.toMatchObject({
      activeLinks: 5,
      adopted: 0,
      guards: 5,
      hidden: 5,
    });

    // Every root waits for its own native link. Only roots still hidden by
    // their guard switch to the shared sheet; the first root stays native.
    releaseStylesheet();
    await expect.poll(() => inspect(page, 'test-cold-link-card')).toEqual({
      activeLinks: 1,
      adopted: 4,
      cards: 5,
      colors: ['rgb(44, 55, 66)'],
      distinctSheets: 1,
      guards: 0,
      hidden: 0,
      links: 5,
      sameSheetAsFirstBatch: true,
    });
  });

  test('keeps native links when the stylesheet cannot be promoted', async ({ page }) => {
    await openFixture(page);
    await register(page, 'test-import-link-card', true);

    // Later links for the same URL may load after this task, so each root is
    // guarded until its own native link is ready, then stays native.
    const native = {
      adopted: 0,
      colors: ['rgb(77, 88, 99)'],
      distinctSheets: 0,
      guards: 0,
      hidden: 0,
    };
    await mountNow(page, 'test-import-link-card', 3);
    await expect.poll(() => inspect(page, 'test-import-link-card')).toMatchObject({
      ...native,
      activeLinks: 3,
      links: 3,
    });
    await mountNow(page, 'test-import-link-card', 2);
    await expect.poll(() => inspect(page, 'test-import-link-card')).toMatchObject({
      ...native,
      activeLinks: 5,
      links: 5,
    });
    await expect.poll(() => page.evaluate(() => {
      const card = document.querySelector('test-import-link-card');
      const label = card?.shadowRoot?.querySelector('.label');
      return label ? getComputedStyle(label).fontStyle : null;
    })).toBe('italic');
  });

  test('promotes eagerly for components with their own template styles', async ({
    page,
  }) => {
    await openFixture(page);
    await register(page, 'test-styled-link-card', true);

    // The adopted catalog sheet cascades after the template's <style>, so the
    // first instance adopts it too and every instance keeps the same order.
    await mountNow(page, 'test-styled-link-card', 3);
    await expect.poll(() => inspect(page, 'test-styled-link-card')).toEqual({
      activeLinks: 0,
      adopted: 3,
      cards: 3,
      colors: ['rgb(101, 102, 103)'],
      distinctSheets: 1,
      guards: 0,
      hidden: 0,
      links: 1,
      sameSheetAsFirstBatch: true,
    });
  });
});
