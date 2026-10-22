// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';
import type {} from './element.js';

const HTML = 'http://www.w3.org/1999/xhtml';
const SVG = 'http://www.w3.org/2000/svg';
const MATH = 'http://www.w3.org/1998/Math/MathML';

async function openFixture(page: Page, enforceTrustedTypes = false): Promise<void> {
  if (enforceTrustedTypes) {
    await page.route('**/foreign-content/fixture.html', async route => {
      const response = await route.fetch();
      await route.fulfill({
        response,
        headers: {
          ...response.headers(),
          'content-security-policy': "require-trusted-types-for 'script'; trusted-types webui",
        },
      });
    });
  }
  await page.goto('/foreign-content/fixture.html');
  await page.waitForFunction(() => {
    const hosts = [...document.querySelectorAll('test-foreign-content')];
    return hosts.length === 2 && hosts.every(host => host.ready);
  });
}

for (const enforced of [false, true]) {
  test(`conditional and repeated SVG shapes paint in their native namespace${enforced ? ' under Trusted Types' : ''}`, async ({ page }) => {
    await openFixture(page, enforced);
    const errors: string[] = [];
    page.on('pageerror', error => errors.push(error.message));
    for (let cycle = 0; cycle < 2; cycle++) {
      for (const button of await page.locator('test-foreign-content .toggle').all()) await button.click();
      await expect(page.locator('test-foreign-content .conditional-shape')).toHaveCount(2);
      const shapes = await page.locator('test-foreign-content .conditional-shape, test-foreign-content .nested-shape, test-foreign-content .point')
        .evaluateAll(elements => elements.map(element => ({
          namespace: element.namespaceURI,
          graphics: element instanceof SVGGraphicsElement,
          width: element.getBoundingClientRect().width,
          height: element.getBoundingClientRect().height,
        })));
      expect(shapes).toHaveLength(8);
      for (const shape of shapes) {
        expect(shape.namespace).toBe(SVG);
        expect(shape.graphics).toBe(true);
        expect(shape.width).toBeGreaterThan(0);
        expect(shape.height).toBeGreaterThan(0);
      }
      const reuse = await page.evaluate(() => {
        const host = document.querySelector('test-foreign-content');
        if (!host?.shadowRoot) throw new Error('Missing test host');
        const before = [...host.shadowRoot.querySelectorAll('.point')];
        host.reverseItems();
        host.$flushUpdates();
        const after = [...host.shadowRoot.querySelectorAll('.point')];
        return before[0] === after[1] && before[1] === after[0];
      });
      expect(reuse).toBe(true);
      for (const button of await page.locator('test-foreign-content .toggle').all()) await button.click();
      await expect(page.locator('test-foreign-content .conditional-shape')).toHaveCount(0);
    }
    expect(errors).toEqual([]);
  });
}

test('one named block can be cloned in HTML and SVG without sharing incompatible cached DOM', async ({ page }) => {
  await openFixture(page);
  for (const button of await page.locator('test-foreign-content .toggle').all()) await button.click();
  await expect(page.locator('test-foreign-content .html-links a')).toHaveCount(2);
  expect(await page.locator('test-foreign-content .html-links a').evaluateAll(elements => elements.map(element => element.namespaceURI)))
    .toEqual([HTML, HTML]);
  expect(await page.locator('test-foreign-content .plot > a').evaluateAll(elements => elements.map(element => element.namespaceURI)))
    .toEqual([SVG, SVG]);
});

test('foreign parsed templates stay inert until their live children are imported', async ({ page }) => {
  await openFixture(page, true);
  for (let cycle = 1; cycle <= 2; cycle++) {
    for (const button of await page.locator('test-foreign-content .toggle').all()) await button.click();
    await expect(page.locator('test-foreign-content test-foreign-child')).toHaveCount(2);
    const constructions = await page.evaluate(() => {
      const child = customElements.get('test-foreign-child');
      if (!child || !('constructions' in child) || typeof child.constructions !== 'number') {
        throw new Error('Missing child constructor count');
      }
      return child.constructions;
    });
    expect(constructions).toBe(cycle * 2);
    for (const button of await page.locator('test-foreign-content .toggle').all()) await button.click();
    await expect(page.locator('test-foreign-content test-foreign-child')).toHaveCount(0);
  }
});

test('foreign integration points retain HTML and MathML children through nested staging and encoding changes', async ({ page }) => {
  await openFixture(page);
  for (const button of await page.locator('test-foreign-content .toggle').all()) await button.click();
  for (const selector of ['.foreign-html', '.math-html']) {
    expect(await page.locator(`test-foreign-content ${selector}`).evaluateAll(elements => elements.map(element => element.namespaceURI)))
      .toEqual([HTML, HTML]);
  }
  for (const selector of ['.math-symbol', '.math-glyph']) {
    expect(await page.locator(`test-foreign-content ${selector}`).evaluateAll(elements => elements.map(element => element.namespaceURI)))
      .toEqual([MATH, MATH]);
  }
  expect(await page.locator('test-foreign-content .math-number').evaluateAll(elements => elements.map(element => element.namespaceURI)))
    .toEqual([MATH, MATH, MATH, MATH]);
  expect(await page.locator('test-foreign-content .annotation-link').evaluateAll(elements => elements.map(element => element.namespaceURI)))
    .toEqual([HTML, MATH]);
  for (const [encoding, expected] of [
    ['APPLICATION/XHTML+XML', HTML], ['text/plain', MATH], ['TEXT/HTML', HTML],
  ]) {
    const namespace = await page.evaluate(next => {
      const host = document.querySelector('test-foreign-content');
      if (!host?.shadowRoot) throw new Error('Missing test host');
      host.show = false;
      host.$flushUpdates();
      host.encoding = next;
      host.show = true;
      host.$flushUpdates();
      return host.shadowRoot.querySelector('.annotation-link')?.namespaceURI;
    }, encoding);
    expect(namespace).toBe(expected);
  }
});
