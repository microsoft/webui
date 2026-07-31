// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('non-bubbling event fixture', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/non-bubbling-event/fixture.html');
    await page.waitForSelector('test-non-bubbling');
  });

  test('native focus/blur bindings fire even though the events do not bubble', async ({ page }) => {
    const errors: string[] = [];
    page.on('pageerror', (err) => errors.push(err.message));

    const result = await page.evaluate(async () => {
      const host = document.querySelector('test-non-bubbling') as any;
      const root = host.shadowRoot ?? host;
      const field = root.querySelector('.field') as HTMLInputElement;
      field.focus();
      field.blur();
      host.$flushUpdates();
      return {
        focuses: host.focuses,
        blurs: host.blurs,
        currentTarget: host.lastCurrentTarget,
        bubbles: new FocusEvent('focus').bubbles,
      };
    });

    // Guards the premise: the browser really does dispatch focus without bubbling.
    expect(result.bubbles).toBe(false);
    expect(result.focuses).toBe(1);
    expect(result.blurs).toBe(1);
    // Direct listeners keep the native currentTarget.
    expect(result.currentTarget).toBe('field');
    expect(errors).toEqual([]);
  });

  test('error and mouseenter bindings fire', async ({ page }) => {
    const result = await page.evaluate(() => {
      const host = document.querySelector('test-non-bubbling') as any;
      const root = host.shadowRoot ?? host;
      root.querySelector('.thumb').dispatchEvent(new Event('error'));
      root.querySelector('.box').dispatchEvent(new MouseEvent('mouseenter'));
      host.$flushUpdates();
      return { errors: host.errors, enters: host.enters };
    });

    expect(result.errors).toBe(1);
    expect(result.enters).toBe(1);
  });

  test('non-bubbling bindings inside a repeat keep their item scope', async ({ page }) => {
    const result = await page.evaluate(() => {
      const host = document.querySelector('test-non-bubbling') as any;
      const root = host.shadowRoot ?? host;
      const rows = root.querySelectorAll('.row-input');
      (rows[1] as HTMLInputElement).focus();
      host.$flushUpdates();
      return { rowCount: rows.length, lastRowId: host.lastRowId };
    });

    expect(result.rowCount).toBe(2);
    expect(result.lastRowId).toBe('b:b');
  });

  test('bubbling bindings fire on their own element', async ({ page }) => {
    const result = await page.evaluate(() => {
      const host = document.querySelector('test-non-bubbling') as any;
      const root = host.shadowRoot ?? host;
      root.querySelector('.btn').dispatchEvent(new MouseEvent('click', { bubbles: true, composed: true }));
      host.$flushUpdates();
      return host.clicks;
    });

    expect(result).toBe(1);
  });

  test('application-defined events fire even when dispatched without bubbles', async ({ page }) => {
    // `new CustomEvent(name)` defaults to bubbles: false, and an app-defined
    // name can never appear in a framework-shipped event list.
    const result = await page.evaluate(() => {
      const host = document.querySelector('test-non-bubbling') as any;
      const root = host.shadowRoot ?? host;
      const widget = root.querySelector('.widget');
      const picked = new CustomEvent('ui-picked');
      widget.dispatchEvent(picked);
      host.$flushUpdates();
      return { picks: host.picks, bubbles: picked.bubbles };
    });

    expect(result.bubbles).toBe(false);
    expect(result.picks).toBe(1);
  });

  test('platform events the framework cannot enumerate fire', async ({ page }) => {
    const result = await page.evaluate(() => {
      const host = document.querySelector('test-non-bubbling') as any;
      const root = host.shadowRoot ?? host;
      root.querySelector('.widget').dispatchEvent(new Event('cuechange'));
      host.$flushUpdates();
      return host.cues;
    });

    expect(result).toBe(1);
  });

  test('a bubbling application-defined event still reaches its binding from a descendant', async ({ page }) => {
    const result = await page.evaluate(() => {
      const host = document.querySelector('test-non-bubbling') as any;
      const root = host.shadowRoot ?? host;
      const widget = root.querySelector('.widget');
      const child = document.createElement('span');
      widget.appendChild(child);
      child.dispatchEvent(new CustomEvent('ui-picked', { bubbles: true }));
      host.$flushUpdates();
      return host.picks;
    });

    expect(result).toBe(1);
  });

  test('a binding fires when an ancestor stops propagation before the render root', async ({ page }) => {
    // A listener on the bound element runs during the target phase, before an
    // ancestor can stop anything. One on the render root would be suppressed.
    const result = await page.evaluate(() => {
      const host = document.querySelector('test-non-bubbling') as any;
      const root = host.shadowRoot ?? host;
      const guard = root.querySelector('.guard') as HTMLElement;
      guard.addEventListener('click', (event: Event) => event.stopPropagation());
      const button = root.querySelector('.guarded') as HTMLButtonElement;
      button.dispatchEvent(new MouseEvent('click', { bubbles: true, composed: true }));
      host.$flushUpdates();
      return host.guardedClicks;
    });

    expect(result).toBe(1);
  });

  test('listeners are removed when the component disconnects', async ({ page }) => {
    const result = await page.evaluate(async () => {
      const host = document.querySelector('test-non-bubbling') as any;
      const root = host.shadowRoot ?? host;
      const thumb = root.querySelector('.thumb') as HTMLImageElement;
      thumb.dispatchEvent(new Event('error'));
      const before = host.errors;

      // Teardown is scheduled on a microtask by disconnectedCallback.
      host.remove();
      await new Promise<void>((resolve) => queueMicrotask(() => resolve()));

      thumb.dispatchEvent(new Event('error'));
      return { before, after: host.errors };
    });

    expect(result.before).toBe(1);
    expect(result.after).toBe(1);
  });
});
