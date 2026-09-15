// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';
import type { TestTrustedTypes } from './src/test-trusted-types/test-trusted-types.js';

test.beforeEach(async ({ page }) => {
  await page.route('**/trusted-types/fixture.html', async route => {
    const response = await route.fetch();
    await route.fulfill({
      response,
      headers: {
        ...response.headers(),
        'content-security-policy': "require-trusted-types-for 'script'; trusted-types review-compiled",
      },
    });
  });
});

test('enforced Trusted Types preserves rejected raw DOM and the rest of its update batch', async ({ page }) => {
  const errors: Error[] = [];
  page.on('pageerror', error => errors.push(error));
  await page.goto('/trusted-types/fixture.html');
  const host = page.locator('test-trusted-types');
  await page.waitForFunction(() => customElements.get('test-trusted-types') !== undefined);
  await expect(host.locator('.raw-initial')).toHaveText('Initial');
  expect(errors).toEqual([]);

  await page.evaluate(() => {
    const element = document.querySelector('test-trusted-types') as TestTrustedTypes;
    element.rawHtml = '<b class="rejected">Must not render</b>';
    element.count = 1;
  });
  await expect(host.locator('.count')).toHaveText('1');
  await expect(host.locator('.raw-initial')).toHaveText('Initial');
  await expect(host.locator('.rejected')).toHaveCount(0);
  expect(errors).toHaveLength(1);
  expect(errors[0].name).toBe('TypeError');
  expect(errors[0].message).toContain('TrustedHTML');

  await page.evaluate(() => {
    const element = document.querySelector('test-trusted-types') as TestTrustedTypes;
    element.count = 2;
    element.show = false;
    element.items = ['First', 'Second'];
  });
  await expect(host.locator('.count')).toHaveText('2');
  await expect(host.locator('.conditional')).toHaveCount(0);
  await expect(host.locator('.item')).toHaveText(['First', 'Second']);
  await page.evaluate(() => {
    (document.querySelector('test-trusted-types') as TestTrustedTypes).show = true;
  });
  await expect(host.locator('.conditional')).toHaveText('Visible');
  expect(errors).toHaveLength(1);
});

test('the policy exposes no global trust methods and client-created compiler blocks still mount', async ({ page }) => {
  const errors: Error[] = [];
  page.on('pageerror', error => errors.push(error));
  await page.goto('/trusted-types/fixture.html');
  await page.waitForFunction(() => customElements.get('test-trusted-types') !== undefined);

  const boundary = await page.evaluate(() => {
    const descriptor = Object.getOwnPropertyDescriptor(window, '__webuiTrustedTypesPolicyName');
    const rejected = (operation: () => void): boolean => {
      try {
        operation();
        return false;
      } catch (error) {
        if (error instanceof TypeError) return true;
        throw error;
      }
    };
    return {
      descriptor,
      hasBridge: Object.hasOwn(window, '__webuiTrustedTemplates'),
      rawHTMLRejected: rejected(() => {
        document.createElement('template').innerHTML = '<p>Untrusted</p>';
      }),
      rawScriptRejected: rejected(() => {
        document.createElement('script').textContent = 'window.untrustedScriptRan = true';
      }),
    };
  });
  expect(boundary).toEqual({
    descriptor: { value: 'review-compiled', writable: false, enumerable: false, configurable: false },
    hasBridge: false,
    rawHTMLRejected: true,
    rawScriptRejected: true,
  });

  await page.evaluate(() => {
    const element = document.createElement('test-trusted-types') as TestTrustedTypes;
    element.id = 'client-created';
    element.rawHtml = '';
    element.count = 3;
    element.show = true;
    element.items = ['Client'];
    document.body.appendChild(element);
  });
  const created = page.locator('#client-created');
  await expect(created.locator('.count')).toHaveText('3');
  await expect(created.locator('.conditional')).toHaveText('Visible');
  await expect(created.locator('.item')).toHaveText('Client');
  expect(errors).toEqual([]);
});
