// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('interaction hydration boundary', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/interaction-hydration/fixture.html');
    await page.waitForFunction(() => window.interactionFixture !== undefined);
  });

  test('replays an SVG-origin click after listeners are ready', async ({ page }) => {
    const accepted = await page.evaluate(() => {
      const target = document.querySelector('#icon-path');
      return target?.dispatchEvent(new MouseEvent('click', {
        bubbles: true,
        cancelable: true,
        composed: true,
      }));
    });
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
    const checkedBeforeHydration = await page.evaluate(() => {
      const checkbox = document.querySelector<HTMLInputElement>('#toggle');
      checkbox?.click();
      return checkbox?.checked;
    });
    expect(checkedBeforeHydration).toBe(false);

    await page.evaluate(() => window.releaseInteractionHydration());
    await expect(page.locator('#toggle')).toBeChecked();
    expect(
      await page.evaluate(() => window.interactionFixture.appClicks),
    ).toBe(1);
  });

  test('pointerdown starts loading without cancellation', async ({ page }) => {
    const outcome = await page.evaluate(() => {
      const target = document.querySelector('#icon-button');
      const event = new PointerEvent('pointerdown', {
        bubbles: true,
        button: 0,
        cancelable: true,
        composed: true,
        isPrimary: true,
        pointerId: 1,
        pointerType: 'mouse',
      });
      return {
        accepted: target?.dispatchEvent(event),
        defaultPrevented: event.defaultPrevented,
      };
    });

    expect(outcome).toEqual({
      accepted: true,
      defaultPrevented: false,
    });
    await expect.poll(
      () => page.evaluate(() => window.interactionFixture.loadCount),
    ).toBe(1);
  });

  test('focus starts loading without changing focus', async ({ page }) => {
    await page.locator('#text-input').focus();

    await expect(page.locator('#text-input')).toBeFocused();
    await expect.poll(
      () => page.evaluate(() => window.interactionFixture.loadCount),
    ).toBe(1);
  });

  test('keydown starts loading without cancellation', async ({ page }) => {
    const outcome = await page.evaluate(() => {
      const target = document.querySelector('#icon-button');
      const event = new KeyboardEvent('keydown', {
        bubbles: true,
        cancelable: true,
        composed: true,
        key: 'Enter',
      });
      return {
        accepted: target?.dispatchEvent(event),
        defaultPrevented: event.defaultPrevented,
      };
    });

    expect(outcome).toEqual({
      accepted: true,
      defaultPrevented: false,
    });
    await expect.poll(
      () => page.evaluate(() => window.interactionFixture.loadCount),
    ).toBe(1);
  });

  test('keyboard input remains native while hydration is pending', async ({ page }) => {
    await page.locator('#text-input').focus();
    await page.keyboard.type('a');

    await expect(page.locator('#text-input')).toHaveValue('a');
  });

  test('modified and unclonable clicks pass through unchanged', async ({ page }) => {
    const outcomes = await page.evaluate(() => {
      const target = document.querySelector('#icon-button');
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
        modifiedAccepted: target?.dispatchEvent(modified),
        modifiedPrevented: modified.defaultPrevented,
        unclonableAccepted: target?.dispatchEvent(unclonable),
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

  test('does not replay a click cancelled by an earlier capture listener', async ({ page }) => {
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
