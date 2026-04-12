// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Regression test: client-created components (no SSR) must render
 * their initial observable values immediately.
 */

import { expect, test } from '@playwright/test';

test.describe('client-wire: client-created components render initial values', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/client-wire/fixture.html');
    await page.waitForFunction(() => {
      const el = document.querySelector('#host') as any;
      return el && el.$ready === true;
    });
  });

  test('renders initial observable values in shadow DOM', async ({ page }) => {
    const result = await page.evaluate(() => {
      const host = document.querySelector('#host') as any;
      const greeting = host?.shadowRoot?.querySelector('.greeting');
      const count = host?.shadowRoot?.querySelector('.count');
      return {
        greeting: greeting?.textContent,
        count: count?.textContent,
      };
    });

    expect(result.greeting).toBe('Hello');
    expect(result.count).toBe('42');
  });

  test('updates reactively after initial render', async ({ page }) => {
    await page.evaluate(() => {
      const host = document.querySelector('#host') as any;
      host.greeting = 'World';
      host.count = 99;
    });

    await page.waitForFunction(() => {
      const host = document.querySelector('#host') as any;
      return host?.shadowRoot?.querySelector('.greeting')?.textContent === 'World';
    });

    const result = await page.evaluate(() => {
      const host = document.querySelector('#host') as any;
      return {
        greeting: host?.shadowRoot?.querySelector('.greeting')?.textContent,
        count: host?.shadowRoot?.querySelector('.count')?.textContent,
      };
    });

    expect(result.greeting).toBe('World');
    expect(result.count).toBe('99');
  });

  test('initial values are available synchronously after appendChild', async ({ page }) => {
    // Create a NEW element and verify values are flushed immediately
    const result = await page.evaluate(() => {
      const el = document.createElement('test-client-wire') as any;
      document.body.appendChild(el);

      // Check IMMEDIATELY — no await
      const greeting = el.shadowRoot?.querySelector('.greeting');
      const count = el.shadowRoot?.querySelector('.count');
      return {
        greeting: greeting?.textContent,
        count: count?.textContent,
        ready: el.$ready,
      };
    });

    expect(result.ready).toBe(true);
    expect(result.greeting).toBe('Hello');
    expect(result.count).toBe('42');
  });
});
