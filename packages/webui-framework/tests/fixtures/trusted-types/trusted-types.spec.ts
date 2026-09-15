// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test, type Page } from '@playwright/test';
import type { TestTrustedTypes } from './element.js';

async function serveWithCsp(page: Page, policy: string): Promise<void> {
  await page.route('**/trusted-types/fixture.html', async route => {
    const response = await route.fetch();
    await route.fulfill({
      response,
      headers: {
        ...response.headers(),
        'content-security-policy': policy,
      },
    });
  });
}

test.beforeEach(async ({ page }) => {
  await serveWithCsp(page, "require-trusted-types-for 'script'; trusted-types webui");
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
      hasPolicyState: Object.hasOwn(window, '__webuiTrustedTypesPolicyName'),
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
    hasPolicyState: false,
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

test('without enforcement raw strings retain their normal behavior', async ({ page }) => {
  await page.unroute('**/trusted-types/fixture.html');
  const errors: Error[] = [];
  page.on('pageerror', error => errors.push(error));
  await page.goto('/trusted-types/fixture.html');
  await page.waitForFunction(() => customElements.get('test-trusted-types') !== undefined);
  await page.evaluate(() => {
    const element = document.querySelector('test-trusted-types') as TestTrustedTypes;
    element.rawHtml = '<b class="raw-updated">Updated</b>';
    element.count = 1;
  });
  await expect(page.locator('test-trusted-types .raw-updated')).toHaveText('Updated');
  await expect(page.locator('test-trusted-types .count')).toHaveText('1');
  expect(errors).toEqual([]);
});

for (const enforcement of ['', "require-trusted-types-for 'script'; "]) {
  test(`policy-name denial surfaces an error with ${enforcement ? 'enforced' : 'unenforced'} sinks`, async ({ page }) => {
    await serveWithCsp(page, `${enforcement}trusted-types 'none'`);
    const errors: Error[] = [];
    page.on('pageerror', error => errors.push(error));
    await page.goto('/trusted-types/fixture.html');
    await expect.poll(() => errors.some(error => error.message.includes('Allow "webui" in CSP'))).toBe(true);
    await expect(page.locator('test-trusted-types .raw-initial')).toHaveText('Initial');
  });
}
