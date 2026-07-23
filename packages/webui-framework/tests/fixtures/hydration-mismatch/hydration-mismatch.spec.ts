// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Regression test for issue #379.
 *
 * When an observable is assigned at or before `super.connectedCallback()` to a
 * value that differs from the server-rendered DOM, the framework trusts the SSR
 * content (it does NOT reconcile) but emits a dev-facing hydration-mismatch
 * warning — the same signal React/Vue/Svelte/Solid produce. Assignments made
 * after `super.connectedCallback()`, deferred assignments, and pre-ready
 * assignments that match seeded SSR state must stay silent.
 */

import { expect, test } from '@playwright/test';

const MISMATCH = 'Hydration mismatch';

const ALL_TAGS = [
  'mismatch-field-default',
  'mismatch-constructor',
  'mismatch-before-super',
  'mismatch-after-super',
  'mismatch-deferred',
  'mismatch-seeded',
] as const;

test.describe('hydration-mismatch (#379)', () => {
  let warnings: string[];

  test.beforeEach(async ({ page }) => {
    warnings = [];
    page.on('console', (msg) => {
      if (msg.type() === 'warning') warnings.push(msg.text());
    });

    await page.addInitScript((tags) => {
      const snapshots: Array<{ hydratedTags: string[]; readyState: DocumentReadyState }> = [];
      (window as unknown as { hydrationSnapshots: typeof snapshots }).hydrationSnapshots = snapshots;
      window.addEventListener('webui:hydration-complete', () => {
        snapshots.push({
          hydratedTags: tags.filter((tag) => {
            return (document.querySelector(tag) as { $ready?: boolean } | null)?.$ready === true;
          }),
          readyState: document.readyState,
        });
      });
    }, ALL_TAGS as unknown as string[]);

    await page.goto('/hydration-mismatch/fixture.html');

    await page.waitForFunction((tags) => {
      return tags.every((t) => (document.querySelector(t) as { $ready?: boolean } | null)?.$ready === true);
    }, ALL_TAGS as unknown as string[]);

    // The deferred component assigns in a post-ready task; wait for it to land.
    await page.waitForSelector('mismatch-deferred .content');
  });

  const warningsFor = (tag: string): string[] =>
    warnings.filter((w) => w.includes(MISMATCH) && w.includes(`<${tag}>`));

  test('warns exactly once per pre-ready timing and stays silent otherwise', async () => {
    await expect.poll(() => warnings.filter((w) => w.includes(MISMATCH)).length).toBe(3);

    expect(warningsFor('mismatch-field-default')).toHaveLength(1);
    expect(warningsFor('mismatch-constructor')).toHaveLength(1);
    expect(warningsFor('mismatch-before-super')).toHaveLength(1);

    expect(warningsFor('mismatch-after-super')).toHaveLength(0);
    expect(warningsFor('mismatch-deferred')).toHaveLength(0);
    expect(warningsFor('mismatch-seeded')).toHaveLength(0);
  });

  test('the warning names the mismatched observables', async () => {
    await expect.poll(() => warningsFor('mismatch-constructor').length).toBe(1);
    const [warning] = warningsFor('mismatch-constructor');
    expect(warning).toContain('"show"');
    expect(warning).toContain('"value"');
    expect(warning).toContain('super.connectedCallback()');
  });

  test('SSR DOM is trusted, not reconciled, for pre-ready mismatches', async ({ page }) => {
    for (const tag of ['mismatch-field-default', 'mismatch-constructor', 'mismatch-before-super']) {
      // Conditional content stays absent (server rendered it empty).
      expect(await page.locator(`${tag} .content`).count()).toBe(0);
      // Bound attribute keeps the server value, not the client "3".
      expect(await page.locator(`${tag} .box`).getAttribute('data-value')).not.toBe('3');
    }
  });

  test('element state holds the client value even though the DOM does not (the #379 divergence)', async ({ page }) => {
    const state = await page.evaluate(() => {
      const el = document.querySelector('mismatch-field-default') as { show?: boolean; value?: string } | null;
      return { show: el?.show, value: el?.value };
    });
    expect(state).toEqual({ show: true, value: '3' });
    expect(await page.locator('mismatch-field-default .content').count()).toBe(0);
  });

  test('post-super and deferred assignments update the DOM', async ({ page }) => {
    for (const tag of ['mismatch-after-super', 'mismatch-deferred']) {
      await expect(page.locator(`${tag} .content`)).toHaveText('CONTENT');
      expect(await page.locator(`${tag} .box`).getAttribute('data-value')).toBe('3');
    }
  });

  test('super.connectedCallback hydrates synchronously while the document is loading', async ({ page }) => {
    const lifecycle = await page.evaluate(() => {
      const el = document.querySelector('mismatch-after-super') as {
        readyStateAtConnect?: string;
        referencesReadyAfterSuper?: boolean;
      } | null;
      return {
        readyStateAtConnect: el?.readyStateAtConnect,
        referencesReadyAfterSuper: el?.referencesReadyAfterSuper,
      };
    });

    expect(lifecycle).toEqual({
      readyStateAtConnect: 'loading',
      referencesReadyAfterSuper: true,
    });
  });

  test('global completion waits for parsing and every initial hydration', async ({ page }) => {
    const snapshots = await page.evaluate(() => {
      return (window as unknown as {
        hydrationSnapshots?: Array<{ hydratedTags: string[]; readyState: DocumentReadyState }>;
      }).hydrationSnapshots;
    });

    expect(snapshots).toHaveLength(1);
    expect(snapshots?.[0]?.hydratedTags).toEqual([...ALL_TAGS]);
    expect(snapshots?.[0]?.readyState).not.toBe('loading');
  });

  test('seeded pre-ready values match the server render and do not warn', async ({ page }) => {
    await expect(page.locator('mismatch-seeded .content')).toHaveText('CONTENT');
    expect(await page.locator('mismatch-seeded .box').getAttribute('data-value')).toBe('3');
    expect(warningsFor('mismatch-seeded')).toHaveLength(0);
  });
});
