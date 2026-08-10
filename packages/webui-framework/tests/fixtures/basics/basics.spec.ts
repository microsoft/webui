// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Consolidated E2E tests for basic framework features:
 * text bindings, @attr, @observable, click events, derived values,
 * SSR hydration, and client-created component mounting.
 *
 * Replaces the former text, counter, event, and client-wire fixtures.
 */

import { expect, test } from '@playwright/test';

test.describe('basics: SSR hydration', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/basics/fixture.html');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-basics');
      return el && (el as any).$ready === true;
    });
  });

  test('renders SSR text content', async ({ page }) => {
    await expect(page.locator('test-basics .greeting')).toHaveText('Hello');
    await expect(page.locator('test-basics .name')).toHaveText('World');
    await expect(page.locator('test-basics .count')).toHaveText('0');
    await expect(page.locator('test-basics .doubled')).toHaveText('0');
  });

  test('fires the hydration completion event', async ({ page }) => {
    const fired = await page.evaluate(() =>
      performance.getEntriesByName('webui:hydrate:total', 'measure').length > 0,
    );
    expect(fired).toBe(true);
  });
});

test.describe('basics: reactive updates', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/basics/fixture.html');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-basics');
      return el && (el as any).$ready === true;
    });
  });

  test('updates @attr text reactively', async ({ page }) => {
    await page.evaluate(() => {
      document.querySelector('test-basics')?.setAttribute('greeting', 'Hi');
    });
    await expect(page.locator('test-basics .greeting')).toHaveText('Hi');
  });

  test('updates @observable text reactively', async ({ page }) => {
    await page.evaluate(() => {
      (document.querySelector('test-basics') as any).name = 'WebUI';
    });
    await expect(page.locator('test-basics .name')).toHaveText('WebUI');
  });

  test('updates derived @observable on click', async ({ page }) => {
    await page.locator('test-basics .inc').click();
    await page.locator('test-basics .inc').click();
    await expect(page.locator('test-basics .count')).toHaveText('2');
    await expect(page.locator('test-basics .doubled')).toHaveText('4');
  });
});

test.describe('basics: click events', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/basics/fixture.html');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-basics');
      return el && (el as any).$ready === true;
    });
  });

  test('increments on click', async ({ page }) => {
    await page.locator('test-basics .inc').click();
    await expect(page.locator('test-basics .count')).toHaveText('1');
  });

  test('decrements after multiple increments', async ({ page }) => {
    await page.locator('test-basics .inc').click();
    await page.locator('test-basics .inc').click();
    await page.locator('test-basics .dec').click();
    await expect(page.locator('test-basics .count')).toHaveText('1');
  });

  test('resets to zero', async ({ page }) => {
    await page.locator('test-basics .inc').click();
    await page.locator('test-basics .inc').click();
    await page.locator('test-basics .reset').click();
    await expect(page.locator('test-basics .count')).toHaveText('0');
    await expect(page.locator('test-basics .doubled')).toHaveText('0');
  });
});

test.describe('basics: client-created component', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/basics/fixture.html');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-basics');
      return el && (el as any).$ready === true;
    });
  });

  test('renders initial values when created via createElement', async ({ page }) => {
    const result = await page.evaluate(() => {
      const el = document.createElement('test-basics') as any;
      document.body.appendChild(el);
      return {
        greeting: el.shadowRoot?.querySelector('.greeting')?.textContent,
        count: el.shadowRoot?.querySelector('.count')?.textContent,
        ready: el.$ready,
      };
    });
    expect(result.ready).toBe(true);
    expect(result.greeting).toBe('Hello');
    expect(result.count).toBe('0');
  });

  test('client-created component updates reactively', async ({ page }) => {
    await page.evaluate(() => {
      const el = document.createElement('test-basics') as any;
      el.id = 'dynamic';
      document.body.appendChild(el);
    });
    await page.waitForFunction(() => {
      return (document.querySelector('#dynamic') as any)?.$ready === true;
    });

    await page.evaluate(() => {
      (document.querySelector('#dynamic') as any).name = 'Dynamic';
    });
    await expect(page.locator('#dynamic .name')).toHaveText('Dynamic');
  });
});

test.describe('basics: @input and @keydown events', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/basics/fixture.html');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-basics');
      return el && (el as any).$ready === true;
    });
  });

  // TODO: @input/@keydown events compile correctly in the template but
  // the event listener doesn't fire after SSR hydration. This may be a
  // path resolution mismatch between the template h-string and the SSR
  // shadow root DOM. Tracking for investigation.
  test.fixme('@input fires on text entry', async ({ page }) => {
    await page.evaluate(() => {
      const host = document.querySelector('test-basics') as any;
      const input = host?.shadowRoot?.querySelector('.text-input') as HTMLInputElement;
      input.value = 'typed';
      input.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await expect(page.locator('test-basics .input-value')).toHaveText('typed');
  });

  test.fixme('@keydown captures key presses', async ({ page }) => {
    await page.evaluate(() => {
      const host = document.querySelector('test-basics') as any;
      const input = host?.shadowRoot?.querySelector('.text-input') as HTMLInputElement;
      input.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }));
    });
    await expect(page.locator('test-basics .last-key')).toHaveText('ArrowDown');
  });
});

test.describe('browser parser quirks', () => {
  // The compiler numbers elements in pre-order and the browser resolves those
  // numbers against its own parse of the template. Two constructs make the two
  // trees disagree unless the compiler accounts for them: a bare <tr> gains an
  // implied <tbody>, and <template> content is parked off the light tree.
  // Either one shifts every index after it.
  test.beforeEach(async ({ page }) => {
    await page.goto('/basics/fixture.html');
    await page.waitForFunction(() => {
      const el = document.querySelector('test-parser-quirks');
      return el && (el as unknown as { $ready?: boolean }).$ready === true;
    });
  });

  test('binds through an implied tbody and a parked template', async ({ page }) => {
    const host = page.locator('test-parser-quirks');

    await expect(host.locator('.cell')).toHaveText('cell');
    await expect(host.locator('.after')).toHaveText('after');

    // A binding after both constructs must still land on its own element.
    await page.evaluate(() => {
      (document.querySelector('test-parser-quirks') as unknown as { cellValue: string })
        .cellValue = 'updated';
    });
    await expect(host.locator('.cell')).toHaveText('updated');
    await expect(host.locator('.after')).toHaveText('after');

    // And the event target after them must be the button, not a neighbour.
    await host.locator('.after').click();
    await expect(host.locator('.after')).toHaveText('bumped');
    await expect(host.locator('.cell')).toHaveText('updated');
  });
});
