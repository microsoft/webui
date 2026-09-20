// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('interaction hydration boundary', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/interaction-hydration/fixture.html');
    await page.waitForFunction(() => window.interactionFixture !== undefined);
  });

  test('combines compiler-owned render and interaction policy', async ({ page }) => {
    expect(await page.locator('test-interaction').evaluate((root) => ({
      contentVisibility: getComputedStyle(root).contentVisibility,
      policy: window.__webui?.templates?.['test-interaction']?.wp,
    }))).toEqual({
      contentVisibility: 'auto',
      policy: 4,
    });
  });

  test('replays an SVG-origin click after listeners are ready', async ({ page }) => {
    const accepted = await page.locator('#icon-path').evaluate((target) =>
      target.dispatchEvent(new MouseEvent('click', {
        bubbles: true,
        cancelable: true,
        composed: true,
      }))
    );
    expect(accepted).toBe(false);

    await page.evaluate(() => window.releaseInteractionHydration());
    await expect.poll(
      () => page.evaluate(() => window.interactionFixture.appClicks),
    ).toBe(1);
    expect(await page.evaluate(() => window.interactionFixture)).toEqual({
      appClicks: 1,
      documentCaptureReplays: [false, true],
      loadCount: 1,
      replayed: true,
      targetId: 'icon-path',
    });
  });

  test('replayed checkbox click performs native activation once', async ({ page }) => {
    expect(await page.locator('#toggle').evaluate((element) => {
      (element as HTMLInputElement).click();
      return (element as HTMLInputElement).checked;
    })).toBe(false);

    await page.evaluate(() => window.releaseInteractionHydration());
    await expect(page.locator('#toggle')).toBeChecked();
    expect(
      await page.evaluate(() => window.interactionFixture.appClicks),
    ).toBe(1);
  });

  for (const kind of ['pointerdown', 'focus', 'keydown'] as const) {
    test(`${kind} starts loading without cancellation`, async ({ page }) => {
      const outcome = await page.evaluate((wakeKind) => {
        const host = document.querySelector('test-interaction');
        const root = host?.shadowRoot ?? host;
        const target = root?.querySelector<HTMLElement>(
          wakeKind === 'focus' ? '#text-input' : '#icon-button',
        );
        if (!target) throw new Error('wake target is missing');
        if (wakeKind === 'focus') {
          target.focus();
          return { accepted: true, defaultPrevented: false };
        }
        const event = wakeKind === 'pointerdown'
          ? new PointerEvent(wakeKind, {
              bubbles: true,
              button: 0,
              cancelable: true,
              composed: true,
              isPrimary: true,
              pointerId: 1,
              pointerType: 'mouse',
            })
          : new KeyboardEvent(wakeKind, {
              bubbles: true,
              cancelable: true,
              composed: true,
              key: 'Enter',
            });
        return {
          accepted: target.dispatchEvent(event),
          defaultPrevented: event.defaultPrevented,
        };
      }, kind);
      expect(outcome).toEqual({ accepted: true, defaultPrevented: false });
      await expect.poll(
        () => page.evaluate(() => window.interactionFixture.loadCount),
      ).toBe(1);

      if (kind === 'focus') {
        await expect(page.locator('#text-input')).toBeFocused();
        await page.keyboard.type('a');
        await expect(page.locator('#text-input')).toHaveValue('a');
      }
    });
  }

  test('modified and unclonable clicks pass through unchanged', async ({ page }) => {
    const outcomes = await page.evaluate(() => {
      const host = document.querySelector('test-interaction');
      const target = (host?.shadowRoot ?? host)?.querySelector('#icon-button');
      if (!target) throw new Error('click target is missing');
      const modified = new MouseEvent('click', {
        bubbles: true,
        cancelable: true,
        ctrlKey: true,
      });
      const unclonable = new Event('click', {
        bubbles: true,
        cancelable: true,
      });
      return {
        modifiedAccepted: target.dispatchEvent(modified),
        modifiedPrevented: modified.defaultPrevented,
        unclonableAccepted: target.dispatchEvent(unclonable),
        unclonablePrevented: unclonable.defaultPrevented,
      };
    });
    expect(outcomes).toEqual({
      modifiedAccepted: true,
      modifiedPrevented: false,
      unclonableAccepted: true,
      unclonablePrevented: false,
    });
  });

  test('does not replay an earlier capture cancellation', async ({ page }) => {
    await page.locator('#blocked-link').click();
    expect(new URL(page.url()).hash).toBe('');

    await page.evaluate(() => window.releaseInteractionHydration());
    await expect.poll(
      () => page.evaluate(() => window.interactionFixture.loadCount),
    ).toBe(1);
    expect(new URL(page.url()).hash).toBe('');
    expect(
      await page.evaluate(
        () => window.interactionFixture.documentCaptureReplays,
      ),
    ).toEqual([false]);
  });
});
