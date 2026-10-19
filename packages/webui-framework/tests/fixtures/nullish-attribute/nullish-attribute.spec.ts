// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test, type Locator } from '@playwright/test';
import type { TestNullishAttribute } from './element.js';

async function expectCases(root: Locator): Promise<void> {
  for (const [name, value] of [
    ['missing', null],
    ['null', null],
    ['empty', ''],
    ['zero', '0'],
    ['false', 'false'],
    ['string', '[a-z]+'],
  ] as const) {
    const input = root.locator(`[data-case="${name}"]`);
    if (value === null) {
      await expect(input).not.toHaveAttribute('pattern');
    } else {
      await expect(input).toHaveAttribute('pattern', value);
    }
    await expect(input).toHaveValue('abc');
  }
  await expect(root.locator('[data-case="missing"]')).toHaveAttribute('title', 'prefix--suffix');
  await expect(root.locator('[data-case="string"]')).toHaveAttribute('title', 'prefix-[a-z]+-suffix');
}

test('SSR omits only nullish whole-binding attributes without JavaScript', async ({ browser }) => {
  const context = await browser.newContext({ javaScriptEnabled: false });
  try {
    const page = await context.newPage();
    await page.goto('/nullish-attribute/fixture.html');
    await expectCases(page.locator('#entry'));
    await expectCases(page.locator('test-nullish-attribute'));
    const validity = await page.locator('#entry input').evaluateAll(inputs =>
      inputs.map(input => (input as HTMLInputElement).validity.patternMismatch));
    expect(validity).toEqual([false, false, true, true, true, false]);
  } finally {
    await context.close();
  }
});

test('hydration preserves omitted attributes, existing nodes, and sibling bindings', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.route('**/dist/nullish-attribute/element.js', async route => {
    await page.waitForFunction(() =>
      document.querySelector('test-nullish-attribute')?.shadowRoot?.querySelector('#live'));
    await page.evaluate(() => {
      const input = document.querySelector('test-nullish-attribute')!.shadowRoot!.querySelector('#live')!;
      Object.assign(window, { nullishSSRInput: input });
    });
    await route.continue();
  });
  await page.goto('/nullish-attribute/fixture.html');
  const host = page.locator('test-nullish-attribute');
  await expect.poll(() => host.evaluate(el => (el as unknown as { $ready: boolean }).$ready)).toBe(true);
  await expectCases(host);
  await expect(host.locator('#live')).not.toHaveAttribute('pattern');
  await expect(host.locator('#missing')).not.toHaveAttribute('pattern');
  expect(await host.locator('#live').evaluate(input =>
    input === (window as Window & { nullishSSRInput?: Element }).nullishSSRInput)).toBe(true);
  await host.locator('button').click();
  await expect(host.locator('#live')).toHaveAttribute('data-label', 'After');
  await expect(host.locator('button')).toHaveText('After');
  expect(errors).toEqual([]);
});

test('reactive updates remove nullish attributes and preserve falsy values', async ({ page }) => {
  await page.goto('/nullish-attribute/fixture.html');
  const host = page.locator('test-nullish-attribute');
  await expect.poll(() => host.evaluate(el => (el as unknown as { $ready: boolean }).$ready)).toBe(true);
  const input = host.locator('#live');
  for (const value of ['[a-z]+', null, '', 0, false, undefined, '[0-9]+'] as const) {
    await host.evaluate((el, constraint) => {
      (el as TestNullishAttribute).constraint = constraint;
    }, value);
    if (value == null) {
      await expect(input).not.toHaveAttribute('pattern');
    } else {
      await expect(input).toHaveAttribute('pattern', String(value));
    }
    expect(await input.evaluate(el => (el as HTMLInputElement).validity.patternMismatch))
      .toBe(value != null && value !== '[a-z]+');
  }
  await input.fill('user edit');
  await host.evaluate(el => { (el as TestNullishAttribute).inputValue = null; });
  await expect(input).toHaveValue('');
  await expect(input).not.toHaveAttribute('value');
});

test('client-created templates omit nullish attributes on their first binding pass', async ({ page }) => {
  await page.goto('/nullish-attribute/fixture.html');
  await page.evaluate(() => {
    const host = document.createElement('test-nullish-attribute') as TestNullishAttribute;
    host.id = 'client-created';
    document.body.appendChild(host);
  });
  const host = page.locator('#client-created');
  await expect.poll(() => host.evaluate(el => (el as unknown as { $ready: boolean }).$ready)).toBe(true);
  await expect(host.locator('#live')).not.toHaveAttribute('pattern');
  await expect(host.locator('#missing')).not.toHaveAttribute('pattern');
  await expect(host.locator('#live')).toHaveValue('abc');
});
