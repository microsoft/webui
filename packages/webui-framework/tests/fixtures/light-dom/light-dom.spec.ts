// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { expect, test } from '@playwright/test';

test.describe('light-dom pipeline', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/light-dom/fixture.html');
    await expect(page.locator('#light-root')).toHaveJSProperty('$ready', true);
    await expect(page.locator('#shadow-opt-in')).toHaveJSProperty('$ready', true);
  });

  test('omitted DOM option defaults to Light and applies scoped CSS', async ({ page }) => {
    await expect(page.locator('test-light-dom .greeting')).toHaveText('Hello');
    await expect(page.locator('test-light-dom .name')).toHaveText('World');
    await expect(page.locator('test-light-dom .greeting')).toHaveCSS(
      'color',
      'rgb(12, 34, 56)',
    );

    const result = await page.locator('#light-root').evaluate((host) => ({
      hasShadow: !!host.shadowRoot,
      lightMarker: host.hasAttribute('data-wl'),
      scopedCss: Array.from(
        document.head.querySelectorAll('style[data-webui-resource]'),
        style => style.textContent,
      ).some(css => css?.includes('@scope (test-light-dom[data-wl])')),
    }));
    expect(result).toEqual({
      hasShadow: false,
      lightMarker: true,
      scopedCss: true,
    });
  });

  test('installs the Light parent/child closure in order and deduplicates client mounts', async ({ page }) => {
    const resourceIds = () => page.locator(
      'head > [data-webui-resource]',
    ).evaluateAll(elements => elements.map(
      element => element.getAttribute('data-webui-resource'),
    ));

    expect(await resourceIds()).toEqual(['test-light-dom', 'test-light-child']);
    await expect(page.locator('#light-root > test-light-child')).toHaveCount(2);

    await page.locator('#light-root').evaluate((host) => {
      const spawnChild = Reflect.get(host, 'spawnChild');
      if (typeof spawnChild !== 'function') {
        throw new Error('Missing spawnChild()');
      }
      Reflect.apply(spawnChild, host, []);
    });

    await expect(page.locator('.client-children > test-light-child')).toHaveCount(1);
    await expect(page.locator('.client-children .child-label')).toHaveCSS(
      'color',
      'rgb(34, 139, 34)',
    );
    expect(await resourceIds()).toEqual(['test-light-dom', 'test-light-child']);
  });

  test('Shadow opt-in cuts the Document closure and projects a native slot', async ({ page }) => {
    const result = await page.locator('#shadow-opt-in').evaluate((host) => {
      const root = host.shadowRoot;
      const slot = root?.querySelector('slot');
      return {
        hasShadow: !!root,
        lightMarker: host.hasAttribute('data-wl'),
        projected: slot instanceof HTMLSlotElement
          ? slot.assignedElements().map(element => element.textContent?.trim())
          : [],
        resourceIds: root
          ? Array.from(root.children)
            .filter(element => element.hasAttribute('data-webui-resource'))
            .map(element => element.getAttribute('data-webui-resource'))
          : [],
        nestedLightMarkers: root?.querySelectorAll(
          'test-shadow-light-child[data-wl]',
        ).length ?? 0,
        nestedColor: (() => {
          const label = root?.querySelector('.nested-label');
          return label instanceof HTMLElement ? getComputedStyle(label).color : null;
        })(),
      };
    });

    expect(result).toEqual({
      hasShadow: true,
      lightMarker: false,
      projected: ['Projected label'],
      resourceIds: ['test-shadow-opt-in', 'test-shadow-light-child'],
      nestedLightMarkers: 2,
      nestedColor: 'rgb(70, 130, 180)',
    });
    await expect(page.locator('#shadow-opt-in > .projected')).toHaveCSS(
      'color',
      'rgb(128, 0, 128)',
    );

    const documentResources = await page.locator(
      'head > [data-webui-resource]',
    ).evaluateAll(elements => elements.map(
      element => element.getAttribute('data-webui-resource'),
    ));
    expect(documentResources).not.toContain('test-shadow-opt-in');
    expect(documentResources).not.toContain('test-shadow-light-child');
  });
});
